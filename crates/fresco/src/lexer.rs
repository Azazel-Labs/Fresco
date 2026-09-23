//! Lexer for Fresco, built on `logos`.
//!
//! Design notes:
//! - Newlines are significant (they separate statements and compose entries),
//!   so they are a token rather than skipped whitespace.
//! - Numeric literals carry their unit suffix (`px`, `uv`, `deg`, `turn`, `s`, `ms`) out of
//!   the lexer, so `12px` is one token. The f64 payload is stored as raw bits
//!   (`u64`) so that `Token` can derive `Eq + Hash`, which chumsky's error
//!   type requires.
//! - Color literals `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa` are packed to an
//!   RGBA8888 `u32` in the lexer; malformed ones become lex errors.

use logos::Logos;
use smol_str::SmolStr;
use std::ops::Range;

pub const KEYWORD_LEXEMES: &[&str] = &[
    "canvas",
    "surface",
    "pass",
    "pipeline",
    "tags",
    "when",
    "import",
    "enum",
    "struct",
    "interface",
    "conform",
    "extern",
    "fn",
    "internal",
    "for",
    "if",
    "else",
    "let",
    "param",
    "in",
    "space",
    "canvas_space",
    "compose",
    "style",
    "blend",
    "scatter",
    "within",
    "seed",
    "strategy",
    "lifetime",
    "respawn",
    "every",
    "field",
    "layer",
    "at",
    "return",
    "break",
    "vertex",
    "texture_type",
    "effect",
    "rewrite",
    "s",
    "ms",
];

pub const UNIT_SUFFIXES: &[&str] = &[
    "px", "uv", "vw", "vh", "vmin", "vmax", "deg", "turn", "s", "ms", "/s", "/ms",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenHighlightKind {
    Keyword,
    Operator,
    Number,
    String,
    Identifier,
    Comment,
}

impl TokenHighlightKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Keyword => "keyword",
            Self::Operator => "operator",
            Self::Number => "number",
            Self::String => "string",
            Self::Identifier => "identifier",
            Self::Comment => "comment",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unit {
    None,
    Px,
    Uv,
    Vw,
    Vh,
    Vmin,
    Vmax,
    Deg,
    Turn,
    Sec,
    MilliSec,
}

fn parse_num(lex: &mut logos::Lexer<'_, Token>) -> Option<(u64, Unit)> {
    parse_number(lex.slice())
}

fn parse_typed_num(lex: &mut logos::Lexer<'_, Token>) -> Option<(u64, bool)> {
    let source = lex.slice();
    let number: f64 = source[..source.len() - 1].parse().ok()?;
    Some((number.to_bits(), source.ends_with('u')))
}

fn parse_number(s: &str) -> Option<(u64, Unit)> {
    let (num, unit) = if let Some(n) = s.strip_suffix("deg") {
        (n, Unit::Deg)
    } else if let Some(n) = s.strip_suffix("turn") {
        (n, Unit::Turn)
    } else if let Some(n) = s.strip_suffix("px") {
        (n, Unit::Px)
    } else if let Some(n) = s.strip_suffix("uv") {
        (n, Unit::Uv)
    } else if let Some(n) = s.strip_suffix("vw") {
        (n, Unit::Vw)
    } else if let Some(n) = s.strip_suffix("vh") {
        (n, Unit::Vh)
    } else if let Some(n) = s.strip_suffix("vmin") {
        (n, Unit::Vmin)
    } else if let Some(n) = s.strip_suffix("vmax") {
        (n, Unit::Vmax)
    } else if let Some(n) = s.strip_suffix("ms") {
        (n, Unit::MilliSec)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, Unit::Sec)
    } else if let Some(n) = s.strip_suffix('u') {
        (n, Unit::None)
    } else if let Some(n) = s.strip_suffix('i') {
        (n, Unit::None)
    } else {
        (s, Unit::None)
    };
    let v: f64 = num.parse().ok()?;
    Some((v.to_bits(), unit))
}

fn parse_rate(lex: &mut logos::Lexer<'_, Token>) -> Option<(u64, Unit, bool)> {
    let (numerator, denominator) = lex.slice().split_once('/')?;
    // Do not accept a valid-looking prefix of an unknown unit such as /second.
    if lex
        .remainder()
        .starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
    {
        return None;
    }
    let (bits, unit) = parse_number(numerator)?;
    Some((bits, unit, denominator == "ms"))
}

fn parse_color(lex: &mut logos::Lexer<'_, Token>) -> Option<u32> {
    let hex = &lex.slice()[1..];
    let nib = |c: u8| -> Option<u32> { (c as char).to_digit(16) };
    let bytes = hex.as_bytes();
    let (r, g, b, a) = match bytes.len() {
        3 | 4 => {
            let d = |i: usize| -> Option<u32> {
                let n = nib(bytes[i])?;
                Some(n << 4 | n) // expand nibble: f -> ff
            };
            let a = if bytes.len() == 4 { d(3)? } else { 0xff };
            (d(0)?, d(1)?, d(2)?, a)
        }
        6 | 8 => {
            let d = |i: usize| -> Option<u32> { Some(nib(bytes[i])? << 4 | nib(bytes[i + 1])?) };
            let a = if bytes.len() == 8 { d(6)? } else { 0xff };
            (d(0)?, d(2)?, d(4)?, a)
        }
        _ => return None,
    };
    Some(r << 24 | g << 16 | b << 8 | a)
}

fn parse_string(lex: &mut logos::Lexer<'_, Token>) -> Option<String> {
    let s = lex.slice();
    let inner = s.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let esc = chars.next()?;
            let mapped = match esc {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                '"' => '"',
                '\\' => '\\',
                _ => return None,
            };
            out.push(mapped);
        } else {
            out.push(ch);
        }
    }
    Some(out)
}

fn parse_triple_string(lex: &mut logos::Lexer<'_, Token>) -> Option<String> {
    let s = lex.slice();
    let inner = s.strip_prefix("\"\"\"")?.strip_suffix("\"\"\"")?;
    Some(inner.to_string())
}

#[derive(Logos, Debug, Clone, PartialEq, Eq, Hash)]
#[logos(skip r"[ \t\r\u{feff}]+")]
pub enum Token {
    #[token("\n")]
    Newline,

    #[regex(r"/\*([^*]|\*+[^*/])*\*+/", logos::skip, allow_greedy = true)]
    #[regex(r"//[^\r\n]*", logos::skip, allow_greedy = true)]
    Comment,

    // Keywords
    #[token("canvas")]
    Canvas,
    #[token("surface")]
    Surface,
    #[token("pass")]
    Pass,
    #[token("pipeline")]
    Pipeline,
    #[token("tags")]
    Tags,
    #[token("when")]
    When,
    #[token("import")]
    Import,
    #[token("enum")]
    Enum,
    #[token("struct")]
    Struct,
    #[token("interface")]
    Interface,
    #[token("conform")]
    Conform,
    #[token("extern")]
    Extern,
    #[token("fn")]
    Fn,
    #[token("internal")]
    Internal,
    #[token("for")]
    For,
    #[token("if")]
    If,
    #[token("else")]
    Else,
    #[token("let")]
    Let,
    #[token("param")]
    Param,
    #[token("in")]
    In,
    #[token("space")]
    Space,
    #[token("canvas_space")]
    CanvasSpace,
    #[token("compose")]
    Compose,
    #[token("style")]
    Style,
    #[token("blend")]
    Blend,
    #[token("scatter")]
    Scatter,
    #[token("within")]
    Within,
    #[token("seed")]
    Seed,
    #[token("strategy")]
    Strategy,
    #[token("lifetime")]
    Lifetime,
    #[token("respawn")]
    Respawn,
    #[token("every")]
    Every,
    #[token("field")]
    Field,
    #[token("layer")]
    Layer,
    #[token("at")]
    At,
    #[token("return")]
    Return,
    #[token("break")]
    Break,
    #[token("vertex")]
    Vertex,
    #[token("each")]
    Each,
    #[token("texture_type")]
    TextureType,
    #[token("#pragma")]
    Pragma,
    #[token("@builtin")]
    Builtin,
    #[regex(r"@[A-Za-z_][A-Za-z0-9_]*", |lex| SmolStr::new(&lex.slice()[1..]))]
    Attribute(SmolStr),
    #[token("effect")]
    Effect,
    #[token("rewrite")]
    Rewrite,
    #[token("through")]
    Through,
    #[token("required")]
    Required,
    #[token("optional")]
    Optional,

    // Punctuation / operators. Note `|>` must be declared before `|`.
    #[token("->")]
    Arrow,
    #[token("=>")]
    FatArrow,
    #[token("<=")]
    Le,
    #[token(">=")]
    Ge,
    #[token("==")]
    EqEq,
    #[token("!=")]
    NotEq,
    #[token("!")]
    Bang,
    #[token("&&")]
    AndAnd,
    #[token("||")]
    OrOr,
    #[token("<<")]
    Shl,
    #[token(">>")]
    Shr,
    #[token("|>")]
    PipeOp,
    #[token("++")]
    PlusPlus,
    #[token("--")]
    MinusMinus,
    #[token("|")]
    Bar,
    #[token("&")]
    Amp,
    #[token("^")]
    Caret,
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("%")]
    Percent,
    #[token("..")]
    RangeOp,
    #[token(".")]
    Dot,
    #[token(",")]
    Comma,
    #[token(":")]
    Colon,
    #[token("=")]
    Eq,
    #[token("?")]
    Question,
    #[token(";")]
    Semicolon,
    #[token("<")]
    Lt,
    #[token(">")]
    Gt,
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,

    // Literals
    #[regex(r"#[0-9a-fA-F]+", parse_color)]
    Color(u32),
    #[regex(r#""""(?s:.*?)""""#, parse_triple_string)]
    #[regex(r#"\"([^\"\\]|\\.)*\""#, parse_string)]
    Str(String),
    /// f64 bits + unit suffix. Bits (not f64) so Token is Eq + Hash.
    #[regex(
        r"[0-9]+(\.[0-9]+)?([eE][+-]?[0-9]+)?(px|uv|vw|vh|vmin|vmax|deg|turn|ms|s|u|i)?",
        parse_num
    )]
    Num((u64, Unit)),

    /// Explicit shader integer suffix, retained through deferred hook parsing.
    #[regex(r"[0-9]+[ui]", parse_typed_num, priority = 4)]
    TypedNum((u64, bool)),

    /// Numerator bits, numerator unit, and whether the denominator is milliseconds.
    #[regex(
        r"[0-9]+(\.[0-9]+)?([eE][+-]?[0-9]+)?(px|uv|vw|vh|vmin|vmax|deg|turn|ms|s|u|i)?/(ms|s)",
        parse_rate
    )]
    Rate((u64, Unit, bool)),

    #[token("s", |lex| SmolStr::new(lex.slice()), priority = 3)]
    #[token("ms", |lex| SmolStr::new(lex.slice()))]
    ReservedTimeUnit(SmolStr),

    #[regex(r"[A-Za-z_][A-Za-z0-9_]*", |lex| SmolStr::new(lex.slice()))]
    Ident(SmolStr),
    #[regex(r"\$[A-Za-z_][A-Za-z0-9_]*", |lex| SmolStr::new(lex.slice()))]
    HostIdent(SmolStr),
}

impl Token {
    pub fn highlight_kind(&self) -> Option<TokenHighlightKind> {
        match self {
            Self::Canvas
            | Self::Surface
            | Self::Pass
            | Self::Pipeline
            | Self::Tags
            | Self::When
            | Self::Import
            | Self::Enum
            | Self::Struct
            | Self::Interface
            | Self::Conform
            | Self::Extern
            | Self::Fn
            | Self::Internal
            | Self::If
            | Self::Else
            | Self::Let
            | Self::Param
            | Self::In
            | Self::Space
            | Self::CanvasSpace
            | Self::Compose
            | Self::Blend
            | Self::Scatter
            | Self::Within
            | Self::Seed
            | Self::Strategy
            | Self::Lifetime
            | Self::Respawn
            | Self::Every
            | Self::Field
            | Self::Layer
            | Self::At
            | Self::Return
            | Self::Break
            | Self::Vertex
            | Self::Each
            | Self::TextureType
            | Self::Pragma
            | Self::Builtin
            | Self::Attribute(_)
            | Self::Effect
            | Self::Rewrite
            | Self::Through
            | Self::Required
            | Self::Optional
            | Self::For
            | Self::Style
            | Self::ReservedTimeUnit(_) => Some(TokenHighlightKind::Keyword),
            Self::Num(_) | Self::TypedNum(_) | Self::Rate(_) | Self::Color(_) => {
                Some(TokenHighlightKind::Number)
            }
            Self::Str(_) => Some(TokenHighlightKind::String),
            Self::Arrow
            | Self::FatArrow
            | Self::PipeOp
            | Self::Bar
            | Self::Amp
            | Self::Caret
            | Self::Plus
            | Self::Minus
            | Self::Star
            | Self::Slash
            | Self::Percent
            | Self::RangeOp
            | Self::Eq
            | Self::Lt
            | Self::Gt
            | Self::Le
            | Self::Ge
            | Self::EqEq
            | Self::NotEq
            | Self::Bang
            | Self::AndAnd
            | Self::OrOr
            | Self::Shl
            | Self::Shr
            | Self::PlusPlus
            | Self::MinusMinus
            | Self::Question => Some(TokenHighlightKind::Operator),
            Self::Ident(_) | Self::HostIdent(_) => Some(TokenHighlightKind::Identifier),
            Self::Comment => Some(TokenHighlightKind::Comment),
            Self::Newline
            | Self::Dot
            | Self::Comma
            | Self::Colon
            | Self::Semicolon
            | Self::LParen
            | Self::RParen
            | Self::LBrace
            | Self::RBrace
            | Self::LBracket
            | Self::RBracket => None,
        }
    }
}

impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use Token::*;
        match self {
            Newline => write!(f, "newline"),
            Canvas => write!(f, "`canvas`"),
            Surface => write!(f, "`surface`"),
            Pass => write!(f, "`pass`"),
            Pipeline => write!(f, "`pipeline`"),
            Tags => write!(f, "`tags`"),
            When => write!(f, "`when`"),
            Import => write!(f, "`import`"),
            Enum => write!(f, "`enum`"),
            Struct => write!(f, "`struct`"),
            Interface => write!(f, "`interface`"),
            Conform => write!(f, "`conform`"),
            Extern => write!(f, "`extern`"),
            Fn => write!(f, "`fn`"),
            Internal => write!(f, "`internal`"),
            For => write!(f, "`for`"),
            If => write!(f, "`if`"),
            Else => write!(f, "`else`"),
            Let => write!(f, "`let`"),
            Param => write!(f, "`param`"),
            In => write!(f, "`in`"),
            Space => write!(f, "`space`"),
            CanvasSpace => write!(f, "`canvas_space`"),
            Compose => write!(f, "`compose`"),
            Style => write!(f, "`style`"),
            Blend => write!(f, "`blend`"),
            Scatter => write!(f, "`scatter`"),
            Within => write!(f, "`within`"),
            Seed => write!(f, "`seed`"),
            Strategy => write!(f, "`strategy`"),
            Lifetime => write!(f, "`lifetime`"),
            Respawn => write!(f, "`respawn`"),
            Every => write!(f, "`every`"),
            Field => write!(f, "`field`"),
            Layer => write!(f, "`layer`"),
            At => write!(f, "`at`"),
            Return => write!(f, "`return`"),
            Break => write!(f, "`break`"),
            Vertex => write!(f, "`vertex`"),
            Each => write!(f, "`each`"),
            TextureType => write!(f, "`texture_type`"),
            Pragma => write!(f, "`#pragma`"),
            Builtin => write!(f, "`@builtin`"),
            Attribute(name) => write!(f, "`@{name}`"),
            Effect => write!(f, "`effect`"),
            Rewrite => write!(f, "`rewrite`"),
            Through => write!(f, "`through`"),
            Required => write!(f, "`required`"),
            Optional => write!(f, "`optional`"),
            Arrow => write!(f, "`->`"),
            FatArrow => write!(f, "`=>`"),
            Le => write!(f, "`<=`"),
            Ge => write!(f, "`>=`"),
            EqEq => write!(f, "`==`"),
            NotEq => write!(f, "`!=`"),
            Bang => write!(f, "`!`"),
            AndAnd => write!(f, "`&&`"),
            OrOr => write!(f, "`||`"),
            Shl => write!(f, "`<<`"),
            Shr => write!(f, "`>>`"),
            PipeOp => write!(f, "`|>`"),
            PlusPlus => write!(f, "`++`"),
            MinusMinus => write!(f, "`--`"),
            Bar => write!(f, "`|`"),
            Amp => write!(f, "`&`"),
            Caret => write!(f, "`^`"),
            Plus => write!(f, "`+`"),
            Minus => write!(f, "`-`"),
            Star => write!(f, "`*`"),
            Slash => write!(f, "`/`"),
            Percent => write!(f, "`%`"),
            RangeOp => write!(f, "`..`"),
            Dot => write!(f, "`.`"),
            Comma => write!(f, "`,`"),
            Colon => write!(f, "`:`"),
            Eq => write!(f, "`=`"),
            Question => write!(f, "`?`"),
            Semicolon => write!(f, "`;`"),
            Lt => write!(f, "`<`"),
            Gt => write!(f, "`>`"),
            LParen => write!(f, "`(`"),
            RParen => write!(f, "`)`"),
            LBrace => write!(f, "`{{`"),
            RBrace => write!(f, "`}}`"),
            LBracket => write!(f, "`[`"),
            RBracket => write!(f, "`]`"),
            Comment => write!(f, "comment"),
            Color(_) => write!(f, "color literal"),
            Str(_) => write!(f, "string literal"),
            Num(_) => write!(f, "number"),
            TypedNum((_, unsigned)) => write!(
                f,
                "{} integer literal",
                if *unsigned { "unsigned" } else { "signed" }
            ),
            Rate(_) => write!(f, "per-time literal"),
            ReservedTimeUnit(unit) => write!(
                f,
                "reserved time unit `{unit}` (use an attached `/s` or `/ms` suffix)"
            ),
            Ident(s) => write!(f, "`{s}`"),
            HostIdent(s) => write!(f, "`{s}`"),
        }
    }
}

/// Best-effort lexical tokenization with byte ranges for editor tooling.
/// Invalid lexemes are skipped so consumers can still paint partial documents.
pub fn lex_spanned(source: &str) -> Vec<(Token, Range<usize>)> {
    Token::lexer(source)
        .spanned()
        .filter_map(|(tok, span)| tok.ok().map(|token| (token, span)))
        .collect()
}
