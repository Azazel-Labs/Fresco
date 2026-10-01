//! Resolved native data types. Access is part of pointer identity, not an IR decoration.
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum Access {
    Read,
    ReadWrite,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum Type {
    Bool,
    I32,
    U32,
    F32,
    Record(String),
    Array(Box<Type>, u32),
    Pointer { pointee: Box<Type>, access: Access },
}

impl Type {
    pub fn parse(source: &str) -> Result<Self, String> {
        let source = source.trim();
        if let Some(base) = source.strip_suffix('*') {
            return Ok(Self::Pointer {
                pointee: Box::new(Self::parse(base)?),
                access: Access::ReadWrite,
            });
        }
        if let Some((name, tail)) = source.split_once('<') {
            let tail = tail
                .strip_suffix('>')
                .ok_or("unterminated type arguments")?;
            let mut depth = 0_i32;
            let mut start = 0;
            let mut args = Vec::new();
            for (i, c) in tail.char_indices() {
                match c {
                    '<' => depth += 1,
                    '>' => depth -= 1,
                    ',' if depth == 0 => {
                        args.push(tail[start..i].trim());
                        start = i + 1;
                    }
                    _ => {}
                }
                if depth < 0 {
                    return Err("unbalanced type arguments".into());
                }
            }
            if depth != 0 {
                return Err("unbalanced type arguments".into());
            }
            args.push(tail[start..].trim());
            return match (name.trim(), args.as_slice()) {
                ("array", [element, count]) => {
                    let count = count.parse::<u32>().map_err(|_| "array length must be a positive u32")?;
                    if count == 0 { return Err("array length must be positive".into()); }
                    Ok(Self::Array(Box::new(Self::parse(element)?), count))
                }
                ("Ptr", [pointee]) => Ok(Self::Pointer { pointee: Box::new(Self::parse(pointee)?), access: Access::ReadWrite }),
                ("Ptr", [pointee, access] | [pointee, access, "AddressSpace.Device"]) => {
                    let access = match *access {
                        "Access.Read" => Access::Read,
                        "Access.ReadWrite" => Access::ReadWrite,
                        _ => return Err("pointer access must be Access.Read or Access.ReadWrite".into()),
                    };
                    Ok(Self::Pointer { pointee: Box::new(Self::parse(pointee)?), access })
                }
                ("Ptr", _) => Err("expected Ptr<T, Access.ReadWrite, AddressSpace.Device>; only device pointers are supported".into()),
                ("array", _) => Err("native arrays require a fixed positive length".into()),
                _ => Err(format!("unsupported native generic type `{source}`")),
            };
        }
        Ok(match source {
            "bool" => Self::Bool,
            "i32" => Self::I32,
            "u32" => Self::U32,
            "f32" => Self::F32,
            name if !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') =>
            {
                Self::Record(name.into())
            }
            _ => return Err(format!("unsupported native type `{source}`")),
        })
    }
}
