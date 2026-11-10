use std::sync::LazyLock;

use eddy_model::{HighlightSpan, TokenClass, utf16_len};
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);

const MAX_HIGHLIGHT_BYTES: usize = 64 * 1024;

/// Languages a reader can pick for a code block, as tokens `highlight`
/// takes; the common ones, in display order.
pub fn highlight_languages() -> &'static [&'static str] {
    &[
        "c",
        "cpp",
        "csharp",
        "css",
        "diff",
        "go",
        "html",
        "java",
        "javascript",
        "json",
        "kotlin",
        "lua",
        "makefile",
        "markdown",
        "objc",
        "php",
        "python",
        "ruby",
        "rust",
        "scala",
        "shell",
        "sql",
        "swift",
        "typescript",
        "xml",
        "yaml",
    ]
}

/// Whether a declared language names a grammar that can be highlighted.
pub fn knows_language(language: &str) -> bool {
    find_syntax(language).is_some()
}

/// Classifies tokens of `code` written in `language`. Unknown languages and
/// oversized blocks get no highlighting.
pub fn highlight(code: &str, language: &str) -> Vec<HighlightSpan> {
    if code.len() > MAX_HIGHLIGHT_BYTES {
        return Vec::new();
    }
    let Some(syntax) = find_syntax(language) else {
        return Vec::new();
    };
    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut spans: Vec<HighlightSpan> = Vec::new();
    let mut offset = 0u32;
    for line in LinesWithEndings::from(code) {
        let Ok(ops) = state.parse_line(line, &SYNTAXES) else {
            return Vec::new();
        };
        let mut last = 0usize;
        for (byte, op) in ops {
            if byte > last {
                offset = emit(&mut spans, &line[last..byte], classify(&stack), offset);
                last = byte;
            }
            if stack.apply(&op).is_err() {
                return Vec::new();
            }
        }
        if last < line.len() {
            offset = emit(&mut spans, &line[last..], classify(&stack), offset);
        }
    }
    spans
}

fn find_syntax(language: &str) -> Option<&'static SyntaxReference> {
    let lang = language.trim().to_lowercase();
    let alias = match lang.as_str() {
        "js" | "javascript" | "jsx" | "ts" | "typescript" | "tsx" | "mjs" => "js",
        "sh" | "shell" | "bash" | "zsh" | "console" => "sh",
        "py" | "python3" => "py",
        "rs" => "rs",
        "c++" | "cpp" | "cc" | "hpp" | "cxx" => "cpp",
        "h" | "c" => "c",
        "objc" | "objective-c" => "m",
        "glsl" | "hlsl" | "metal" | "wgsl" | "cuda" | "cu" => "cpp",
        "yml" => "yaml",
        "md" => "markdown",
        "golang" => "go",
        // Grammars the default set lacks borrow the nearest one.
        "kt" | "kotlin" | "kts" => "scala",
        "swift" | "zig" => "rs",
        "dart" => "java",
        "elixir" | "ex" | "exs" | "crystal" => "rb",
        "html" | "xml" | "svg" | "vue" | "svelte" => "html",
        "csharp" | "c#" | "cs" => "cs",
        "toml" | "ini" | "cfg" | "conf" | "properties" | "env" | "dotenv" => "properties",
        "dockerfile" | "docker" | "fish" | "powershell" | "ps1" => "sh",
        "makefile" | "make" | "mk" => "makefile",
        "diff" | "patch" => "diff",
        "tex" | "latex" => "tex",
        "plaintext" | "plain" | "text" | "txt" | "none" | "nohighlight" => return None,
        other => other,
    };
    SYNTAXES
        .find_syntax_by_token(alias)
        .or_else(|| SYNTAXES.find_syntax_by_token(&lang))
}

fn emit(spans: &mut Vec<HighlightSpan>, text: &str, class: Option<TokenClass>, start: u32) -> u32 {
    let end = start + utf16_len(text);
    if let Some(class) = class {
        if text.trim().is_empty() {
            return end;
        }
        if let Some(last) = spans.last_mut()
            && last.class == class
            && last.end == start
        {
            last.end = end;
            return end;
        }
        spans.push(HighlightSpan { start, end, class });
    }
    end
}

fn classify(stack: &ScopeStack) -> Option<TokenClass> {
    for scope in stack.as_slice().iter().rev() {
        if let Some(class) = class_of(*scope) {
            return Some(class);
        }
    }
    None
}

fn class_of(scope: Scope) -> Option<TokenClass> {
    let name = scope.build_string();
    let starts = |p: &str| name.starts_with(p);
    Some(if starts("comment") {
        TokenClass::Comment
    } else if starts("string") {
        TokenClass::String
    } else if starts("constant.numeric") {
        TokenClass::Number
    } else if starts("constant") || starts("support.constant") {
        TokenClass::Constant
    } else if starts("entity.name.function")
        || starts("support.function")
        || starts("meta.function-call") && name.contains("identifier")
    {
        TokenClass::Function
    } else if starts("entity.name.type")
        || starts("entity.name.class")
        || starts("support.type")
        || starts("support.class")
        || starts("storage.type")
    {
        TokenClass::Type
    } else if starts("keyword.operator") {
        TokenClass::Operator
    } else if starts("keyword") || starts("storage") {
        TokenClass::Keyword
    } else if starts("entity.name.tag") {
        TokenClass::Tag
    } else if starts("entity.other.attribute-name") {
        TokenClass::Attribute
    } else if starts("variable.parameter") || starts("variable.other.member") {
        TokenClass::Variable
    } else if starts("punctuation") {
        TokenClass::Punctuation
    } else {
        return None;
    })
}

#[cfg(test)]
mod tests {
    use super::{find_syntax, highlight};
    use eddy_model::TokenClass;

    #[test]
    fn every_guessable_language_has_a_grammar() {
        for profile in crate::guess::PROFILE_LANGUAGES {
            assert!(find_syntax(profile).is_some(), "{profile}");
        }
    }

    #[test]
    fn plain_text_is_not_a_grammar() {
        assert!(find_syntax("plaintext").is_none());
        assert!(highlight("let x = 1", "text").is_empty());
    }

    #[test]
    fn tokens_are_classified() {
        let spans = highlight("let x = 1;", "rust");
        assert!(spans.iter().any(|s| s.start == 0 && s.end == 3));
        assert!(
            spans
                .iter()
                .any(|s| s.class == TokenClass::Number && s.start == 8)
        );
    }
}
