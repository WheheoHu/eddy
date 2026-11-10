//! Guesses the language of a code block that doesn't say which it is in,
//! from the tokens and shapes typical of each language. The guess is only
//! used for highlighting, so a wrong one costs a few odd colors; a block
//! nothing fits stays plain.

use std::sync::LazyLock;

use regex::Regex;

/// A language and the signs of it: each regex that matches adds its weight.
struct Profile {
    /// A token `highlight::find_syntax` knows.
    language: &'static str,
    signs: &'static [(u32, &'static str)],
}

const PROFILES: &[Profile] = &[
    Profile {
        language: "rust",
        signs: &[
            (4, r"\bfn\s+\w+\s*(<[^>]*>)?\s*\("),
            (3, r"\blet\s+mut\b"),
            (3, r"\bimpl(\s*<[^>]*>)?\s+\w+"),
            (3, r"#\[(derive|cfg|test|inline|allow)\b"),
            (3, r"\b\w+!\s*[\(\[\{]"),
            (2, r"\bpub\s+(fn|struct|enum|mod|use|trait|const)\b"),
            (2, r"\b(match|loop|trait|crate|mod)\b"),
            (2, r"&(mut\s+)?(self|str|\[)"),
            (2, r"\b(Some|None|Ok|Err|Vec|String|Option|Result)\b"),
            (2, r"->\s*[\w<>&\[\]\(\)]+\s*\{"),
            (1, r"::"),
            (1, r"\blet\s+\w+\s*(:\s*[\w<>&\[\]]+)?\s*="),
        ],
    },
    Profile {
        language: "python",
        signs: &[
            (
                4,
                r"(?m)^\s*def\s+\w+\s*\(.*\)\s*(->\s*[\w\[\], ]+)?\s*:\s*$",
            ),
            (
                3,
                r"(?m)^\s*(from\s+[\w.]+\s+)?import\s+[\w.]+(\s+as\s+\w+)?\s*$",
            ),
            (
                3,
                r"(?m)^\s*(if|elif|else|for|while|with|try|except|class)\b[^\n]*:\s*$",
            ),
            (3, r"\bself\.\w+"),
            (2, r"\b(None|True|False|lambda|yield|elif|pass)\b"),
            (2, r"\bprint\s*\("),
            (2, r"\b__\w+__\b"),
            (1, r"(?m)^\s*#[^!]"),
        ],
    },
    Profile {
        language: "typescript",
        signs: &[
            (
                4,
                r"\binterface\s+\w+\s*(<[^>]*>)?\s*(extends\s+[\w<>, ]+)?\{",
            ),
            (4, r#"\btype\s+\w+(<[^>]*>)?\s*=\s*[\{\w\|'"]"#),
            (
                3,
                r"[\(,]\s*\w+\??\s*:\s*(string|number|boolean|any|unknown|void|never|\w+\[\]|Promise<)",
            ),
            (
                3,
                r"(?m)^\s*(readonly\s+)?\w+\??\s*:\s*(string|number|boolean|any|unknown|null|undefined|\w+\[\]|Array<|Record<|Promise<|Map<|Set<)[^\n]*;?\s*$",
            ),
            (
                3,
                r"\)\s*:\s*(string|number|boolean|void|Promise<[^>]*>|\w+\[\])\s*(=>|\{)",
            ),
            (
                2,
                r"\b(implements|readonly|declare|namespace|enum|as\s+const|satisfies)\b",
            ),
            (2, r#"\bimport\s+(type\s+)?\{[^}]*\}\s+from\s+['"]"#),
            (2, r"\b(const|let)\s+\w+\s*:\s*\w+"),
            (2, r"\b(const|let|var)\s+\w+\s*=\s*"),
            (2, r"=>"),
            (1, r"\b(export|async|await)\b"),
        ],
    },
    Profile {
        language: "javascript",
        signs: &[
            (3, r"\b(const|let|var)\s+\w+\s*=\s*"),
            (3, r"\bfunction\s*\w*\s*\([^)]*\)\s*\{"),
            (3, r"=>"),
            (3, r"\bconsole\.(log|error|warn)\s*\("),
            (3, r"\b(document|window)\.\w+"),
            (2, r#"\brequire\s*\(\s*['"]"#),
            (2, r#"\bimport\s+.*\s+from\s+['"]"#),
            (2, r"\b(export\s+default|module\.exports)\b"),
            (2, r"===|!=="),
            (2, r"\b(async|await|null|undefined|typeof|new\s+Promise)\b"),
            (1, r"\.then\s*\("),
        ],
    },
    Profile {
        language: "go",
        signs: &[
            (5, r"(?m)^\s*package\s+\w+\s*$"),
            (4, r"\bfunc\s+(\(\w+\s+\*?\w+\)\s+)?\w+\s*\("),
            (4, r":="),
            (3, r"\bfmt\.\w+\("),
            (3, r"\berr\s*!=\s*nil\b"),
            (2, r"\b(go|defer|chan|goroutine|struct|interface)\b"),
            (2, r"\bimport\s*\(\s*\n"),
            (1, r"\b(nil|range)\b"),
        ],
    },
    Profile {
        language: "cpp",
        signs: &[
            (
                4,
                r"#include\s*<(iostream|vector|string|map|memory|algorithm|cstdio|cstdint)>",
            ),
            (4, r"\bstd::\w+"),
            (3, r"\btemplate\s*<"),
            (
                3,
                r"\b(class|struct)\s+\w+\s*(:\s*(public|private|protected)\s+\w+\s*)?\{",
            ),
            (
                3,
                r"\b(nullptr|constexpr|static_cast|dynamic_cast|reinterpret_cast|virtual|override|noexcept)\b",
            ),
            (2, r"\bcout\s*<<|\bcin\s*>>"),
            (2, r"\busing\s+namespace\b"),
            (2, r"\b(auto|const)\s+\w+&?\s*\w*\s*="),
            (1, r"::"),
            (1, r"->"),
        ],
    },
    Profile {
        language: "c",
        signs: &[
            (4, r"#include\s*<\w+\.h>"),
            (4, r"\bint\s+main\s*\("),
            (3, r"\bprintf\s*\("),
            (3, r"\b(malloc|free|sizeof|memcpy|strlen)\s*\("),
            (3, r"\b(uint\d+_t|int\d+_t|size_t|char\s*\*)\b"),
            (2, r"\b(typedef\s+struct|struct\s+\w+\s*\{|#define|NULL)\b"),
            (
                2,
                r"\b(void|int|char|float|double|unsigned|static|const)\s+\*?\w+\s*[\(=;\[]",
            ),
            (1, r"->"),
            (1, r";\s*$"),
        ],
    },
    Profile {
        language: "java",
        signs: &[
            (
                4,
                r"\bpublic\s+(static\s+)?(final\s+)?(class|interface|enum|void|int|String)\b",
            ),
            (4, r"\bSystem\.(out|err)\.print"),
            (3, r"\bimport\s+(java|javax|android|org)\.[\w.]+;"),
            (3, r"@(Override|Autowired|Test|Inject|Bean)\b"),
            (2, r"\bnew\s+\w+(<[^>]*>)?\s*\("),
            (
                2,
                r"\b(private|protected|public)\s+\w+(<[^>]*>)?\s+\w+\s*[;=\(]",
            ),
            (1, r"\b(extends|implements|throws|instanceof)\b"),
        ],
    },
    Profile {
        language: "kotlin",
        signs: &[
            (4, r"\bfun\s+(\w+\.)?\w+\s*(<[^>]*>)?\s*\("),
            (4, r"\b(val|var)\s+\w+\s*(:\s*[\w<>?]+)?\s*="),
            (
                3,
                r"\b(data\s+class|sealed\s+class|object\s+\w+|companion\s+object)\b",
            ),
            (3, r"\bprintln\s*\("),
            (2, r"\?\.|!!|\?:"),
            (2, r"\bwhen\s*(\([^)]*\))?\s*\{"),
            (1, r"\b(override|suspend|lateinit|internal)\b"),
        ],
    },
    Profile {
        language: "swift",
        signs: &[
            (
                4,
                r"\bfunc\s+\w+\s*(<[^>]*>)?\s*\([^)]*\)\s*(async\s+)?(throws\s+)?(->\s*[\w\[\]<>?]+\s*)?\{",
            ),
            (
                4,
                r"\bimport\s+(Foundation|SwiftUI|UIKit|AppKit|Combine|Swift)\b",
            ),
            (
                3,
                r"\b(guard|let|var)\s+\w+\s*(:\s*[\w\[\]<>?]+)?\s*=.*\belse\b|\bguard\b",
            ),
            (
                3,
                r"@(State|Binding|Published|Observable|MainActor|objc|IBOutlet|IBAction|Environment)\b",
            ),
            (3, r"\b(struct|class|enum|extension)\s+\w+\s*:\s*\w+"),
            (2, r"\b(let|var)\s+\w+\s*(:\s*[\w\[\]<>?]+)?\s*="),
            (2, r"\bprint\s*\("),
            (2, r"\bself\.\w+"),
            (1, r"\b(some|any|weak|unowned|inout|nil)\b"),
        ],
    },
    Profile {
        language: "csharp",
        signs: &[
            (4, r"\busing\s+System(\.[\w.]+)?;"),
            (4, r"\bnamespace\s+[\w.]+"),
            (3, r"\bConsole\.Write(Line)?\s*\("),
            (
                3,
                r"\b(public|private|internal)\s+(static\s+)?(async\s+)?(class|void|string|int|Task|var)\b",
            ),
            (2, r"\b(string|bool|var)\s+\w+\s*="),
            (2, r"\b(get|set);|\{\s*get;\s*set;\s*\}"),
            (
                1,
                r"\b(foreach|override|virtual|readonly|LINQ|async|await)\b",
            ),
        ],
    },
    Profile {
        language: "ruby",
        signs: &[
            (4, r"(?m)^\s*def\s+\w+[?!]?(\(.*\))?\s*$"),
            (4, r"(?m)^\s*end\s*$"),
            (3, r"\bputs\s+"),
            (3, r#"\b(require|require_relative)\s+['"]"#),
            (3, r"\bdo\s*\|\w+(,\s*\w+)*\|"),
            (2, r"\battr_(accessor|reader|writer)\b"),
            (2, r"@\w+\s*="),
            (1, r"\.(each|map|select)\b"),
        ],
    },
    Profile {
        language: "php",
        signs: &[
            (6, r"<\?php"),
            (4, r"\$\w+\s*=\s*"),
            (3, r"\becho\s+"),
            (3, r"\$this->\w+"),
            (2, r"\bfunction\s+\w+\s*\("),
            (2, r"->\w+\("),
            (1, r"\b(namespace|use)\s+[\w\\]+;"),
        ],
    },
    Profile {
        language: "shell",
        signs: &[
            (6, r"^#!\s*/(usr/)?bin/(env\s+)?(ba|z|k|da)?sh\b"),
            (4, r"(?m)^\s*\$\s+\w+"),
            (
                3,
                r"(?m)^\s*(sudo\s+)?(apt|apt-get|brew|npm|yarn|pnpm|pip|pip3|cargo|git|docker|kubectl|curl|wget|make|cmake|cd|ls|mkdir|rm|cp|mv|chmod|export|echo|source|systemctl)\b",
            ),
            (3, r"(?m)^\s*(fi|done|esac)\s*$"),
            (3, r"\b(if|elif)\s+\[\[?\s"),
            (2, r"\$\{\w+\}|\$\(\w"),
            (2, r"\s--?[a-z][\w-]*(\s|=|$)"),
            (2, r"\b(then|do)\s*$"),
            (1, r"\|\s*(grep|awk|sed|sort|xargs|head|tail)\b"),
        ],
    },
    Profile {
        language: "sql",
        signs: &[
            (5, r"(?i)\bselect\b[\s\S]+?\bfrom\b"),
            (
                4,
                r"(?i)\b(insert\s+into|create\s+table|alter\s+table|drop\s+table|update\s+\w+\s+set|delete\s+from)\b",
            ),
            (
                3,
                r"(?i)\b(where|group\s+by|order\s+by|inner\s+join|left\s+join|having)\b",
            ),
            (2, r"(?i)\b(primary\s+key|not\s+null|varchar|integer)\b"),
        ],
    },
    Profile {
        language: "html",
        signs: &[
            (5, r"(?i)<!doctype\s+html>|<html\b"),
            (
                4,
                r"(?i)<(div|span|p|a|ul|li|body|head|section|article|nav|header|footer|table|img|form|input|button)\b[^>]*>",
            ),
            (
                3,
                r"(?i)</(div|span|p|a|ul|li|body|head|section|article|table)>",
            ),
            (2, r#"(?i)\b(class|id|href|src)\s*=\s*""#),
            (2, r"(?i)<(script|style|link|meta)\b"),
        ],
    },
    Profile {
        language: "css",
        signs: &[
            (
                4,
                r"(?m)^\s*([.#][\w-]+|[a-z][\w-]*|\*)(\s*[,>+~]\s*[.#]?[\w-]+)*(:{1,2}[\w-]+(\([^)]*\))?)?\s*\{\s*$",
            ),
            (
                4,
                r"(?m)^\s*(margin|padding|color|background(-[\w-]+)?|display|font(-[\w-]+)?|border(-[\w-]+)?|width|height|position|top|left|right|bottom|flex(-[\w-]+)?|grid(-[\w-]+)?|align-[\w-]+|justify-[\w-]+|text-[\w-]+|line-height|opacity|overflow(-[\w-]+)?|z-index|transition|transform|animation|cursor|content|gap|max-[\w-]+|min-[\w-]+|box-[\w-]+|outline|visibility|white-space|letter-spacing|vertical-align|float|clear|list-style(-[\w-]+)?)\s*:\s*[^;{}\n]+;?\s*$",
            ),
            (
                2,
                r"\b\d+(\.\d+)?(px|em|rem|vh|vw|fr)\b|#[0-9a-fA-F]{3,8}\b",
            ),
            (2, r"@(media|import|keyframes|font-face|supports)\b"),
            (1, r"!important"),
        ],
    },
    Profile {
        language: "json",
        signs: &[
            (5, r#"^\s*[\{\[]"#),
            (4, r#""[^"\n]+"\s*:\s*("|\d|true|false|null|\{|\[)"#),
            (2, r#"[\}\]]\s*$"#),
        ],
    },
    Profile {
        language: "yaml",
        signs: &[
            (4, r"(?m)^---\s*$"),
            (3, r#"(?m)^\s*-\s+[\w"']"#),
            (3, r"(?m)^\s*[\w.-]+:\s+[^\n{]+$"),
            (2, r"(?m)^\s*[\w.-]+:\s*$"),
            (1, r"(?m)^\s*#"),
        ],
    },
    Profile {
        language: "toml",
        signs: &[
            (5, r"(?m)^\s*\[[\w.-]+(\.[\w.-]+)*\]\s*$"),
            (
                4,
                r#"(?m)^\s*[\w.-]+\s*=\s*("[^"]*"|\d[\d._]*|true|false|\[.*\]|\{.*\})\s*$"#,
            ),
            (2, r"(?m)^\s*\[\[[\w.-]+\]\]\s*$"),
        ],
    },
    Profile {
        language: "glsl",
        signs: &[
            (
                5,
                r"\b(gl_Position|gl_FragColor|gl_FragCoord|gl_VertexID|gl_InstanceID)\b",
            ),
            (
                4,
                r"\b(uniform|varying|attribute|layout\s*\(|in\s+vec|out\s+vec|precision\s+(high|medium|low)p)\b",
            ),
            (4, r"#version\s+\d+"),
            (
                3,
                r"\b(vec[234]|mat[234]|ivec[234]|sampler2D|samplerCube)\b",
            ),
            (
                2,
                r"\b(texture|texture2D|normalize|dot|cross|mix|clamp|smoothstep)\s*\(",
            ),
            (2, r"\bvoid\s+main\s*\(\s*\)"),
        ],
    },
    Profile {
        language: "hlsl",
        signs: &[
            (5, r"\bSV_\w+\b"),
            (
                4,
                r"\b(cbuffer|Texture2D|SamplerState|RWTexture2D|StructuredBuffer|register\s*\()\b",
            ),
            (3, r"\b(float[234](x[234])?|half[234]|uint[234]|int[234])\b"),
            (
                2,
                r"\b(numthreads|SV_Target|SV_Position|TEXCOORD\d|POSITION|NORMAL)\b",
            ),
            (2, r"\.Sample\s*\("),
        ],
    },
    Profile {
        language: "metal",
        signs: &[
            (
                6,
                r"#include\s*<metal_stdlib>|\busing\s+namespace\s+metal\b",
            ),
            (
                4,
                r"\[\[\s*(buffer|texture|sampler|stage_in|position|vertex_id|thread_position_in_grid)\b",
            ),
            (3, r"\b(kernel|vertex|fragment)\s+\w+\s+\w+\s*\("),
            (
                2,
                r"\b(float[234](x[234])?|half[234]|texture2d<|device\s+|constant\s+)\b",
            ),
        ],
    },
    Profile {
        language: "wgsl",
        signs: &[
            (
                5,
                r"@(vertex|fragment|compute|group|binding|location|builtin|workgroup_size)\b",
            ),
            (4, r"\bvar<(private|storage|uniform|workgroup)[^>]*>"),
            (3, r"\b(vec[234]<|mat[234]x[234]<|texture_2d<|sampler)\b"),
            (3, r"\bfn\s+\w+\s*\([^)]*\)\s*->\s*@?\w+"),
            (2, r"\blet\s+\w+\s*(:\s*[\w<>]+)?\s*="),
        ],
    },
    Profile {
        language: "makefile",
        signs: &[
            (5, r"(?m)^[\w.-]+\s*:(?:\s+[\w./$()-]+)*\s*\n\t"),
            (3, r"\$\((CC|CXX|CFLAGS|LDFLAGS|SRC|OBJ|@|<|\^)\)"),
            (3, r"(?m)^\.PHONY\s*:"),
            (2, r"(?m)^[\w.-]+\s*[:?+]?=\s*"),
        ],
    },
    Profile {
        language: "dockerfile",
        signs: &[
            (6, r"(?m)^FROM\s+[\w./:@-]+"),
            (
                4,
                r"(?m)^(RUN|COPY|ADD|CMD|ENTRYPOINT|WORKDIR|EXPOSE|ENV|ARG|LABEL|USER|VOLUME)\s+",
            ),
        ],
    },
    Profile {
        language: "diff",
        signs: &[
            (6, r"(?m)^@@\s+-\d+(,\d+)?\s+\+\d+(,\d+)?\s+@@"),
            (5, r"(?m)^(---|\+\+\+)\s+(a/|b/|/dev/null)"),
            (4, r"(?m)^diff\s+--git\s+"),
            (2, r"(?m)^[+-][^+-]"),
        ],
    },
    Profile {
        language: "lua",
        signs: &[
            (5, r"\blocal\s+(function\s+)?\w+"),
            (4, r"(?m)^\s*function\s+[\w.:]+\s*\(.*\)\s*$"),
            (4, r"(?m)^\s*end\s*$"),
            (3, r"\b(then|elseif|repeat|until)\b"),
            (2, r"\.\.|~=|\bnil\b"),
        ],
    },
    Profile {
        language: "markdown",
        signs: &[
            (4, r"(?m)^#{1,6}\s+\S"),
            (3, r"(?m)^\s*[-*]\s+\S"),
            (3, r"\[[^\]]+\]\([^)]+\)"),
            (2, r"(?m)^```"),
            (2, r"\*\*[^*]+\*\*"),
        ],
    },
    Profile {
        language: "latex",
        signs: &[
            (6, r"\\(documentclass|usepackage|begin\{document\})\b"),
            (
                4,
                r"\\(section|subsection|chapter|textbf|emph|cite|ref|label|frac|begin|end)\b",
            ),
            (2, r"\$[^$\n]+\$"),
        ],
    },
    Profile {
        language: "haskell",
        signs: &[
            (5, r"(?m)^\s*\w+\s*::\s*.+->"),
            (4, r"(?m)^(module|import)\s+[A-Z][\w.]*"),
            (
                3,
                r"\b(where|let|in|case|of|data|newtype|instance|deriving)\b",
            ),
            (2, r"<-|\s->\s|\$"),
        ],
    },
    Profile {
        language: "scala",
        signs: &[
            (5, r"\b(object|trait|case\s+class)\s+\w+"),
            (4, r"\bdef\s+\w+\s*(\[[^\]]*\])?\s*\([^)]*\)\s*:\s*\w+"),
            (3, r"\b(val|var)\s+\w+\s*(:\s*\w+)?\s*="),
            (2, r"\bimplicit\b|=>"),
        ],
    },
    Profile {
        language: "ocaml",
        signs: &[
            (5, r"\blet\s+rec\s+\w+"),
            (4, r"\bmatch\s+.+\s+with\s*$|\|\s*\w+\s*(\(.*\))?\s*->"),
            (3, r"\b(let|in|fun|function|module|struct|sig|end)\b"),
            (2, r";;"),
        ],
    },
    Profile {
        language: "elixir",
        signs: &[
            (5, r"\bdefmodule\s+[A-Z][\w.]*\s+do\b"),
            (4, r"\b(def|defp|defmacro)\s+\w+[?!]?(\(.*\))?\s+do\b"),
            (3, r"\|>"),
            (2, r"\bIO\.(puts|inspect)\b|:\w+\b"),
        ],
    },
    Profile {
        language: "zig",
        signs: &[
            (5, r"\bconst\s+\w+\s*=\s*@import\s*\("),
            (4, r"\bpub\s+fn\s+\w+\s*\([^)]*\)\s*!?\w+"),
            (3, r"\b(comptime|anytype|undefined|unreachable)\b"),
            (2, r"@\w+\s*\("),
        ],
    },
    Profile {
        language: "clojure",
        signs: &[
            (5, r"\(defn?\s+[\w\-!?]+"),
            (4, r"\(ns\s+[\w.-]+"),
            (
                3,
                r"\((let|fn|if|when|do|map|reduce|println|println|str|cond)\s",
            ),
            (2, r":[\w-]+\b"),
        ],
    },
];

/// The languages a guess can name.
#[cfg(test)]
pub(crate) const PROFILE_LANGUAGES: &[&str] = &[
    "rust",
    "python",
    "typescript",
    "javascript",
    "go",
    "cpp",
    "c",
    "java",
    "kotlin",
    "swift",
    "csharp",
    "ruby",
    "php",
    "shell",
    "sql",
    "html",
    "css",
    "json",
    "yaml",
    "toml",
    "glsl",
    "hlsl",
    "metal",
    "wgsl",
    "makefile",
    "dockerfile",
    "diff",
    "lua",
    "markdown",
    "latex",
    "haskell",
    "scala",
    "ocaml",
    "elixir",
    "zig",
    "clojure",
];

/// A language and its compiled signs, each with its weight.
type Compiled = (&'static str, Vec<(u32, Regex)>);

static COMPILED: LazyLock<Vec<Compiled>> = LazyLock::new(|| {
    PROFILES
        .iter()
        .map(|p| {
            let signs = p
                .signs
                .iter()
                .map(|(w, re)| (*w, Regex::new(re).expect("language sign regex")))
                .collect();
            (p.language, signs)
        })
        .collect()
});

/// Scoring looks at no more than this much of a block: the start tells.
const SAMPLE_BYTES: usize = 6000;

/// The language the code looks most like, when something fits it clearly.
pub fn guess_language(code: &str) -> Option<&'static str> {
    let scores = scores(code)?;
    let (best, top) = scores[0];
    let second = scores.get(1).map(|s| s.1).unwrap_or(0);
    // Enough signs, and clearly more than the runner-up.
    if top < 6 || top < second + 3 && top < second * 3 / 2 {
        return None;
    }
    Some(best)
}

/// Every language's score, highest first.
fn scores(code: &str) -> Option<Vec<(&'static str, u32)>> {
    let trimmed = code.trim();
    if trimmed.len() < 8 {
        return None;
    }
    let mut end = SAMPLE_BYTES.min(trimmed.len());
    while !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    let sample = &trimmed[..end];
    let lines = sample.lines().count().max(1) as u32;

    let mut scores: Vec<(&'static str, u32)> = COMPILED
        .iter()
        .map(|(language, signs)| {
            let score = signs
                .iter()
                .map(|(weight, re)| {
                    // A sign seen on many lines counts more, up to a point.
                    let hits = re.find_iter(sample).take(8).count() as u32;
                    if hits == 0 {
                        0
                    } else {
                        weight * (1 + hits.min(lines.min(4)) / 2)
                    }
                })
                .sum();
            (*language, score)
        })
        .collect();
    scores.sort_by_key(|s| std::cmp::Reverse(s.1));
    Some(scores)
}

#[cfg(test)]
mod tests {
    use super::guess_language;

    #[test]
    fn recognizes_common_languages() {
        let cases = [
            (
                "rust",
                "fn main() {\n    let mut count = 0;\n    for item in items.iter() {\n        count += 1;\n    }\n    println!(\"{}\", count);\n}",
            ),
            (
                "python",
                "import os\n\ndef main():\n    for name in os.listdir('.'):\n        print(name)\n\nif __name__ == '__main__':\n    main()",
            ),
            (
                "javascript",
                "const items = await fetch(url).then(r => r.json());\nitems.forEach((item) => {\n  console.log(item.name);\n});",
            ),
            (
                "typescript",
                "interface User {\n  id: number;\n  name: string;\n}\n\nfunction greet(user: User): string {\n  return `Hello ${user.name}`;\n}",
            ),
            (
                "go",
                "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tx := 42\n\tif err := run(x); err != nil {\n\t\tfmt.Println(err)\n\t}\n}",
            ),
            (
                "c",
                "#include <stdio.h>\n#include <stdlib.h>\n\nint main(int argc, char **argv) {\n    char *buf = malloc(64);\n    printf(\"%s\\n\", buf);\n    return 0;\n}",
            ),
            (
                "cpp",
                "#include <iostream>\n#include <vector>\n\nint main() {\n    std::vector<int> v{1, 2, 3};\n    for (auto x : v) std::cout << x << std::endl;\n}",
            ),
            (
                "java",
                "import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}",
            ),
            (
                "swift",
                "import SwiftUI\n\nstruct ContentView: View {\n    @State private var count = 0\n    var body: some View {\n        Text(\"\\(count)\")\n    }\n}",
            ),
            (
                "shell",
                "$ cargo build --release\n$ ./target/release/app --port 8080",
            ),
            (
                "shell",
                "#!/bin/bash\nset -e\nfor f in *.txt; do\n  echo \"$f\"\ndone",
            ),
            (
                "sql",
                "SELECT id, title FROM items\nWHERE read = 0\nORDER BY published_at DESC\nLIMIT 10;",
            ),
            (
                "html",
                "<!doctype html>\n<html>\n<body>\n  <div class=\"app\"><p>Hello</p></div>\n</body>\n</html>",
            ),
            (
                "css",
                ".reader {\n  max-width: 700px;\n  margin: 0 auto;\n  color: #333;\n}",
            ),
            (
                "json",
                "{\n  \"name\": \"eddy\",\n  \"version\": 1,\n  \"private\": true\n}",
            ),
            (
                "yaml",
                "services:\n  server:\n    image: eddy/server\n    ports:\n      - \"127.0.0.1:8080:8080\"",
            ),
            (
                "toml",
                "[package]\nname = \"eddy\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde = \"1\"",
            ),
            (
                "glsl",
                "#version 330 core\nuniform mat4 mvp;\nin vec3 position;\nvoid main() {\n    gl_Position = mvp * vec4(position, 1.0);\n}",
            ),
            (
                "hlsl",
                "cbuffer Params : register(b0) { float4x4 mvp; };\nfloat4 main(float3 pos : POSITION) : SV_Position {\n    return mul(mvp, float4(pos, 1));\n}",
            ),
            (
                "wgsl",
                "@group(0) @binding(0) var<uniform> mvp: mat4x4<f32>;\n@vertex\nfn vs(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {\n    return mvp * vec4<f32>(pos, 1.0);\n}",
            ),
            (
                "diff",
                "diff --git a/src/main.rs b/src/main.rs\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,3 +1,4 @@\n fn main() {\n+    run();\n }",
            ),
            (
                "dockerfile",
                "FROM rust:1.85\nWORKDIR /app\nCOPY . .\nRUN cargo build --release\nCMD [\"./target/release/app\"]",
            ),
        ];
        for (expected, code) in cases {
            assert_eq!(
                guess_language(code),
                Some(expected),
                "{code}\n{:?}",
                super::scores(code).map(|s| s.into_iter().take(4).collect::<Vec<_>>())
            );
        }
    }

    #[test]
    fn prose_and_short_snippets_stay_plain() {
        assert_eq!(guess_language("hello"), None);
        assert_eq!(
            guess_language(
                "The quick brown fox jumps over the lazy dog.\nAnd then it went home for the night."
            ),
            None
        );
        assert_eq!(
            guess_language("Error: something went wrong\n    at line 12"),
            None
        );
    }
}
