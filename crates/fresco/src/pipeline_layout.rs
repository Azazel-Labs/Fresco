use crate::lexer::{Token, Unit};

pub(crate) fn canonical_layout_identity(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

pub(crate) fn canonical_layout_signature_from_tokens(tokens: &[Token]) -> Option<String> {
    if tokens.is_empty() {
        return None;
    }

    let serialized = tokens
        .iter()
        .map(serialize_layout_signature_token)
        .collect::<Vec<_>>()
        .join(" ");
    Some(
        serialized
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect(),
    )
}

fn serialize_layout_signature_token(token: &Token) -> String {
    match token {
        Token::Ident(value) => match value.to_ascii_lowercase().as_str() {
            "f32" | "f64" | "half" | "i32" | "u32" | "bool" | "vec2" | "vec3" | "vec4" | "mat4"
            | "uniform" | "buffer" | "read" | "readwrite" => value.to_ascii_lowercase(),
            _ => value.to_string(),
        },
        Token::Attribute(value) => format!("@{}", value),
        Token::Num((bits, unit)) => format_number_with_unit(*bits, *unit),
        Token::TypedNum((bits, _)) => format_number_with_unit(*bits, Unit::None),
        Token::Str(value) => format!("\"{}\"", value),
        Token::LParen => "(".to_string(),
        Token::RParen => ")".to_string(),
        Token::LBrace => "{".to_string(),
        Token::RBrace => "}".to_string(),
        Token::LBracket => "[".to_string(),
        Token::RBracket => "]".to_string(),
        Token::Lt => "<".to_string(),
        Token::Gt => ">".to_string(),
        Token::Comma => ",".to_string(),
        Token::Colon => ":".to_string(),
        Token::Dot => ".".to_string(),
        Token::RangeOp => "..".to_string(),
        _ => format!("{token:?}"),
    }
}

fn format_number_with_unit(bits: u64, unit: Unit) -> String {
    let suffix = match unit {
        Unit::None => "",
        Unit::Px => "px",
        Unit::Uv => "uv",
        Unit::Vw => "vw",
        Unit::Vh => "vh",
        Unit::Vmin => "vmin",
        Unit::Vmax => "vmax",
        Unit::Deg => "deg",
        Unit::Turn => "turn",
        Unit::Sec => "s",
        Unit::MilliSec => "ms",
    };
    format!("{}{}", f64::from_bits(bits), suffix)
}

#[cfg(test)]
mod tests {
    use super::{canonical_layout_identity, canonical_layout_signature_from_tokens};
    use crate::lexer::{Token, Unit};

    #[test]
    fn canonical_layout_identity_ignores_case_and_whitespace() {
        assert_eq!(
            canonical_layout_identity(" buffer < U32 > ReadWrite "),
            "buffer<u32>readwrite"
        );
    }

    #[test]
    fn canonical_layout_signature_from_tokens_stabilizes_layout_shape() {
        let tokens = vec![
            Token::Ident("buffer".into()),
            Token::Lt,
            Token::Ident("U32".into()),
            Token::Gt,
            Token::Ident("readwrite".into()),
            Token::LBracket,
            Token::Num((3.0f64.to_bits(), Unit::None)),
            Token::RBracket,
        ];
        assert_eq!(
            canonical_layout_signature_from_tokens(&tokens),
            Some("buffer<u32>readwrite[3]".to_string())
        );
    }
}
