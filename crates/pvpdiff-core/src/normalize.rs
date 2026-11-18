//! Signature and declaration normalization.
//!
//! Comparison is staged. Exact text equality wins first. Then the normalizer
//! removes formatting noise, alpha-renames type variables, sorts top-level
//! constraints, and unqualifies names for instances. A difference that depends
//! on a type synonym, type family, primitive type, or hole is reported as
//! unknown rather than breaking.

use std::collections::{BTreeMap, BTreeSet};

use pvpdiff_parse::{ApiSurface, EntityKind};

/// Names that are not type variables.
const KEYWORDS: &[&str] = &[
    "forall", "type", "data", "newtype", "class", "instance", "where", "family", "deriving",
    "infix", "infixl", "infixr", "pattern", "import", "module", "case", "of", "let", "in", "if",
    "then", "else", "foreign", "safe", "unsafe",
];

/// Type synonyms and families declared by one package version.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Aliases {
    /// Type synonym names.
    pub synonyms: BTreeSet<String>,
    /// Type family names.
    pub families: BTreeSet<String>,
}

impl Aliases {
    /// Collect aliases from one parsed surface.
    pub fn from_surface(surface: &ApiSurface) -> Self {
        let mut aliases = Self::default();
        for module in surface.modules.values() {
            for (key, entity) in &module.entities {
                match key.kind {
                    EntityKind::TypeSynonym => {
                        aliases.synonyms.insert(key.name.clone());
                    }
                    EntityKind::TypeFamily | EntityKind::DataFamily => {
                        aliases.families.insert(key.name.clone());
                    }
                    _ => {
                        let _ = entity;
                    }
                }
            }
        }
        aliases
    }

    /// Whether a text mentions a synonym from this set.
    pub fn mentions_synonym(&self, text: &str) -> bool {
        mentions_any(text, &self.synonyms)
    }

    /// Whether a text mentions a family from this set.
    pub fn mentions_family(&self, text: &str) -> bool {
        mentions_any(text, &self.families)
    }

    /// Whether a text depends on either set.
    pub fn mentions_any(&self, text: &str) -> bool {
        self.mentions_synonym(text) || self.mentions_family(text)
    }
}

/// Outcome of comparing two signatures or declaration heads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Comparison {
    /// The normalized forms are equal.
    Same,
    /// The forms differ with enough confidence to report a change.
    Changed,
    /// The forms differ in a way the data cannot decide.
    Unknown(&'static str),
}

/// Compare two signatures, using each side's alias sets.
pub fn compare_signatures(
    old: &str,
    new: &str,
    old_aliases: &Aliases,
    new_aliases: &Aliases,
) -> Comparison {
    if old.trim() == new.trim() {
        return Comparison::Same;
    }
    let old_normalized = normalize_text(old, true, false);
    let new_normalized = normalize_text(new, true, false);
    if old_normalized == new_normalized {
        return Comparison::Same;
    }
    if old_aliases.mentions_any(old) || new_aliases.mentions_any(new) {
        return Comparison::Unknown("type synonym or family");
    }
    if old.contains('#') || new.contains('#') {
        return Comparison::Unknown("primitive type");
    }
    if has_hole(&old_normalized) || has_hole(&new_normalized) {
        return Comparison::Unknown("type hole");
    }
    Comparison::Changed
}

/// Compare two declaration heads (data, newtype, class, type synonym, or
/// family).
pub fn compare_heads(
    old: &str,
    new: &str,
    old_aliases: &Aliases,
    new_aliases: &Aliases,
) -> Comparison {
    if old.trim() == new.trim() {
        return Comparison::Same;
    }
    let old_normalized = normalize_text(old, false, false);
    let new_normalized = normalize_text(new, false, false);
    if old_normalized == new_normalized {
        return Comparison::Same;
    }
    if old_aliases.mentions_any(old) || new_aliases.mentions_any(new) {
        return Comparison::Unknown("type synonym or family");
    }
    Comparison::Changed
}

/// Normalize an instance line for package-global set comparison. Module
/// qualification is removed because doc builds on different GHC versions move
/// class and type modules.
pub fn normalize_instance(raw: &str) -> String {
    normalize_text(raw, false, true)
}

/// Normalize only the class and head of an instance, ignoring its context.
pub fn normalize_instance_head(instance: &pvpdiff_parse::Instance) -> String {
    normalize_text(
        &format!("{} {}", instance.class, instance.head),
        false,
        true,
    )
}

/// Generic normalization entry point.
pub fn normalize_text(raw: &str, strip_kinds: bool, unqualify: bool) -> String {
    let text = strip_pragmas(raw);
    let mut tokens = tokenize(&text);
    strip_forall(&mut tokens);
    normalize_star(&mut tokens);
    if strip_kinds {
        strip_trivial_kinds(&mut tokens);
    }
    if unqualify {
        unqualify_words(&mut tokens);
    }
    alpha_rename(&mut tokens);
    let rendered = tokens
        .iter()
        .map(Token::render)
        .collect::<Vec<_>>()
        .join(" ");
    sort_context(&collapse_spaces(&rendered))
}

/// Remove any `{-# ... #-}` pragmas. Constructor strictness `!` and `UNPACK`
/// are both removed here: the bang is kept because it is a single symbol, and
/// `UNPACK` is a representation detail.
fn strip_pragmas(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{-#") {
        result.push_str(&rest[..start]);
        let Some(end) = rest[start..].find("#-}") else {
            break;
        };
        rest = &rest[start + end + 3..];
    }
    result.push_str(rest);
    result
}

/// One lexical token in a type expression.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    /// An identifier or qualified name.
    Word(String),
    /// A run of symbol characters.
    Symbol(String),
    /// A string or character literal.
    Literal(String),
}

impl Token {
    fn render(&self) -> String {
        match self {
            Self::Word(value) | Self::Symbol(value) | Self::Literal(value) => value.clone(),
        }
    }
}

fn tokenize(text: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        let current = chars[index];
        if current.is_whitespace() {
            index += 1;
            continue;
        }
        if current == '"' || current == '\'' {
            let quote = current;
            let mut value = String::new();
            value.push(quote);
            index += 1;
            while index < chars.len() {
                let ch = chars[index];
                value.push(ch);
                index += 1;
                if ch == '\\' && index < chars.len() {
                    value.push(chars[index]);
                    index += 1;
                    continue;
                }
                if ch == quote {
                    break;
                }
            }
            tokens.push(Token::Literal(value));
            continue;
        }
        if current.is_ascii_digit() {
            let mut value = String::new();
            while index < chars.len() && chars[index].is_ascii_digit() {
                value.push(chars[index]);
                index += 1;
            }
            tokens.push(Token::Literal(value));
            continue;
        }
        if current.is_alphabetic() || current == '_' {
            let mut value = String::new();
            while index < chars.len() {
                let ch = chars[index];
                let is_word = ch.is_alphanumeric() || ch == '_' || ch == '\'';
                let is_qualifier_dot = ch == '.'
                    && index + 1 < chars.len()
                    && (chars[index + 1].is_alphabetic() || chars[index + 1] == '_')
                    && !value.is_empty();
                if is_word || is_qualifier_dot {
                    value.push(ch);
                    index += 1;
                } else {
                    break;
                }
            }
            tokens.push(Token::Word(value));
            continue;
        }
        if is_symbol_char(current) {
            let mut value = String::new();
            while index < chars.len() && is_symbol_char(chars[index]) {
                value.push(chars[index]);
                index += 1;
            }
            tokens.push(Token::Symbol(value));
            continue;
        }
        tokens.push(Token::Symbol(current.to_string()));
        index += 1;
    }
    tokens
}

fn is_symbol_char(ch: char) -> bool {
    "!#$%&*+./<=>?@\\^|-~:".contains(ch)
}

fn strip_forall(tokens: &mut Vec<Token>) {
    let Some(position) = tokens
        .iter()
        .position(|token| matches!(token, Token::Word(word) if word == "forall"))
    else {
        return;
    };
    let mut depth = 0usize;
    let mut end = None;
    for (index, token) in tokens.iter().enumerate().skip(position + 1) {
        match token {
            Token::Symbol(symbol) if symbol == "(" || symbol == "[" => depth += 1,
            Token::Symbol(symbol) if symbol == ")" || symbol == "]" => {
                depth = depth.saturating_sub(1)
            }
            Token::Symbol(symbol) if symbol == "." && depth == 0 => {
                end = Some(index);
                break;
            }
            _ => {}
        }
    }
    if let Some(end) = end {
        tokens.drain(position..=end);
    }
}

fn normalize_star(tokens: &mut [Token]) {
    for token in tokens {
        if matches!(token, Token::Symbol(symbol) if symbol == "*") {
            *token = Token::Word("Type".to_string());
        }
    }
}

fn strip_trivial_kinds(tokens: &mut Vec<Token>) {
    let mut index = 0usize;
    while index + 4 < tokens.len() {
        let pattern = matches!(
            (&tokens[index], &tokens[index + 1], &tokens[index + 2], &tokens[index + 3], &tokens[index + 4]),
            (
                Token::Symbol(open),
                Token::Word(_),
                Token::Symbol(separator),
                Token::Word(kind),
                Token::Symbol(close)
            ) if open == "(" && separator == "::" && kind == "Type" && close == ")"
        );
        if pattern && is_type_variable(&tokens[index + 1]) {
            let variable = tokens[index + 1].clone();
            tokens.splice(index..=index + 4, [variable]);
            continue;
        }
        index += 1;
    }
}

fn unqualify_words(tokens: &mut [Token]) {
    for token in tokens {
        if let Token::Word(word) = token
            && let Some((_, tail)) = word.rsplit_once('.')
        {
            *word = tail.to_string();
        }
    }
}

fn alpha_rename(tokens: &mut [Token]) {
    let mut names = BTreeMap::new();
    let mut next = 1usize;
    for token in tokens {
        if !is_type_variable(token) {
            continue;
        }
        let Token::Word(word) = token else {
            continue;
        };
        let entry = names.entry(word.clone()).or_insert_with(|| {
            let name = format!("t{next}");
            next += 1;
            name
        });
        *word = entry.clone();
    }
}

fn is_type_variable(token: &Token) -> bool {
    let Token::Word(word) = token else {
        return false;
    };
    if word.contains('.') || word.is_empty() || KEYWORDS.contains(&word.as_str()) {
        return false;
    }
    word.chars()
        .next()
        .is_some_and(|first| first.is_lowercase() || first == '_')
}

fn has_hole(text: &str) -> bool {
    tokenize(text)
        .iter()
        .any(|token| matches!(token, Token::Word(word) if word == "_"))
}

fn sort_context(text: &str) -> String {
    let Some(position) = find_top_level(text, "=>") else {
        return text.to_string();
    };
    let context = text[..position].trim();
    let head = text[position + 2..].trim();
    let context = strip_outer_parens(context);
    let mut items: Vec<&str> = split_top_level(context, ',')
        .into_iter()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect();
    if items.is_empty() {
        return head.to_string();
    }
    items.sort_unstable();
    items.dedup();
    format!("{} => {head}", items.join(", "))
}

fn strip_outer_parens(text: &str) -> &str {
    let trimmed = text.trim();
    if trimmed.starts_with('(') && trimmed.ends_with(')') {
        let inner = &trimmed[1..trimmed.len() - 1];
        if is_balanced(inner) {
            return inner.trim();
        }
    }
    trimmed
}

fn is_balanced(text: &str) -> bool {
    let mut depth = 0usize;
    for ch in text.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => {
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    depth == 0
}

fn find_top_level(text: &str, needle: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let needle = needle.as_bytes();
    let mut depth = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if depth == 0
            && index + needle.len() <= bytes.len()
            && &bytes[index..index + needle.len()] == needle
        {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn split_top_level(text: &str, delimiter: char) -> Vec<&str> {
    let mut depth = 0usize;
    let mut parts = Vec::new();
    let mut start = 0usize;
    for (index, ch) in text.char_indices() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if ch == delimiter && depth == 0 => {
                parts.push(&text[start..index]);
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

fn collapse_spaces(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn mentions_any(text: &str, names: &BTreeSet<String>) -> bool {
    tokenize(text).iter().any(|token| {
        let Token::Word(word) = token else {
            return false;
        };
        let tail = word
            .rsplit_once('.')
            .map_or(word.as_str(), |(_, tail)| tail);
        names.contains(tail)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Check that renaming a type variable and reordering a context compare
    /// equal.
    #[test]
    fn ignores_alpha_renaming_and_context_order() {
        let old = "(Eq a, Show a) => a -> a";
        let new = "(Show b, Eq b) => b -> b";
        assert_eq!(
            compare_signatures(old, new, &Aliases::default(), &Aliases::default()),
            Comparison::Same
        );
    }

    /// Check that a synonym in a differing signature downgrades the comparison
    /// to unknown.
    #[test]
    fn detects_synonym_uncertainty() {
        let mut aliases = Aliases::default();
        aliases.synonyms.insert("Forest".to_string());
        let old = "Graph -> [Vertex] -> Forest Vertex";
        let new = "Graph -> [Vertex] -> [Tree Vertex]";
        assert_eq!(
            compare_signatures(old, new, &aliases, &aliases),
            Comparison::Unknown("type synonym or family")
        );
    }

    /// Check that instance normalization removes module qualification and
    /// alpha-renames type variables.
    #[test]
    fn normalizes_instance_qualification() {
        let old = "instance GHC.Show.Show (Data.Text.Text a)";
        let new = "instance Show (Text b)";
        assert_eq!(normalize_instance(old), normalize_instance(new));
    }

    /// Check that `*` and `Type` normalize to the same token, and that a
    /// redundant forall does not prevent equality.
    #[test]
    fn normalizes_kinds_and_forall() {
        let old = "forall (a :: *). a -> a";
        let new = "(a :: Type) -> (a :: Type)";
        assert_eq!(
            compare_signatures(old, new, &Aliases::default(), &Aliases::default()),
            Comparison::Same
        );
    }

    /// Check that a genuine arity change is reported as changed.
    #[test]
    fn reports_real_signature_change() {
        assert_eq!(
            compare_signatures(
                "Int -> Int",
                "Int -> Int -> Int",
                &Aliases::default(),
                &Aliases::default()
            ),
            Comparison::Changed
        );
    }
}
