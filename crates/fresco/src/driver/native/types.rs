use super::*;

fn round_up(value: u32, alignment: u32, span: &ast::Span) -> Result<u32> {
    value
        .checked_next_multiple_of(alignment)
        .ok_or_else(|| Error::new(span.clone(), "native layout exceeds u32 addressable size"))
}

impl Compiler<'_> {
    pub(super) fn resolve(&mut self, ty: &Type, span: &ast::Span) -> Result<Resolved> {
        if let Some(resolved) = self.types.get(ty) {
            return Ok(resolved.clone());
        }
        if !self.resolving.insert(ty.clone()) {
            return Err(Error::new(
                span.clone(),
                "recursive native data types are not supported by this Naga representation",
            ));
        }
        let mut fields = Vec::new();
        let mut array_stride = None;
        let (inner, size, alignment) = match ty {
            Type::Bool => (naga::TypeInner::Scalar(naga::Scalar::BOOL), 1, 1),
            Type::I32 => (naga::TypeInner::Scalar(naga::Scalar::I32), 4, 4),
            Type::U32 => (naga::TypeInner::Scalar(naga::Scalar::U32), 4, 4),
            Type::F32 => (naga::TypeInner::Scalar(naga::Scalar::F32), 4, 4),
            Type::Pointer { pointee, .. } => {
                if self.target != Target::NativeSpirV {
                    return Err(Error::new(
                        span.clone(),
                        "device pointers require the native SPIR-V target",
                    ));
                }
                let base = self.resolve(pointee, span)?;
                self.requirements.physical_storage_buffer_addresses = true;
                (
                    naga::TypeInner::Pointer {
                        base: base.handle,
                        space: naga::AddressSpace::PhysicalStorage,
                    },
                    8,
                    8,
                )
            }
            Type::Array(element, count) => {
                let base = self.resolve(element, span)?;
                let stride = round_up(base.layout.size, base.layout.alignment, span)?;
                let size = stride.checked_mul(*count).ok_or_else(|| {
                    Error::new(span.clone(), "native array layout exceeds u32 size")
                })?;
                array_stride = Some(stride);
                (
                    naga::TypeInner::Array {
                        base: base.handle,
                        size: naga::ArraySize::Constant(
                            std::num::NonZeroU32::new(*count).ok_or_else(|| {
                                Error::new(span.clone(), "zero-sized native array")
                            })?,
                        ),
                        stride,
                    },
                    size,
                    base.layout.alignment,
                )
            }
            Type::Record(name) => {
                let declaration = self
                    .program
                    .structs
                    .iter()
                    .find(|s| &s.name == name)
                    .ok_or_else(|| {
                        Error::new(span.clone(), format!("unknown native data type `{name}`"))
                    })?
                    .clone();
                if !declaration.attrs.is_empty() {
                    return Err(Error::new(
                        declaration.span,
                        "native record layout attributes are not supported yet",
                    ));
                }
                let mut members = Vec::new();
                let mut size: u32 = 0;
                let mut alignment = 1;
                let mut names = HashSet::new();
                for field in declaration.fields {
                    if !names.insert(field.name.clone()) {
                        return Err(Error::new(field.span, "duplicate native record field"));
                    }
                    if !field.attrs.is_empty()
                        || field.semantic.is_some()
                        || field.default.is_some()
                    {
                        return Err(Error::new(
                            field.span,
                            "native data fields cannot have semantics, defaults or layout attributes",
                        ));
                    }
                    let ty = Type::parse(&field.ty_name)
                        .map_err(|e| Error::new(field.ty_span.clone(), e))?;
                    let resolved = self.resolve(&ty, &field.ty_span)?;
                    size = round_up(size, resolved.layout.alignment, &field.span)?;
                    fields.push(FieldLayout {
                        name: field.name.clone(),
                        offset: size,
                        ty,
                    });
                    members.push(naga::StructMember {
                        name: Some(field.name),
                        ty: resolved.handle,
                        binding: None,
                        offset: size,
                    });
                    size = size.checked_add(resolved.layout.size).ok_or_else(|| {
                        Error::new(field.span, "native record layout exceeds u32 size")
                    })?;
                    alignment = alignment.max(resolved.layout.alignment);
                }
                size = round_up(size, alignment, span)?;
                (
                    naga::TypeInner::Struct {
                        members,
                        span: size,
                    },
                    size,
                    alignment,
                )
            }
        };
        let name = if let Type::Record(name) = ty {
            Some(name.clone())
        } else {
            None
        };
        let handle = self.module.types.insert(naga::Type { name, inner }, SP);
        let result = Resolved {
            handle,
            layout: TypeLayout {
                ty: ty.clone(),
                size,
                alignment,
                array_stride,
                fields,
            },
        };
        self.resolving.remove(ty);
        self.types.insert(ty.clone(), result.clone());
        Ok(result)
    }

    pub(super) fn contains_pointer(&mut self, ty: &Type, span: &ast::Span) -> Result<bool> {
        Ok(match ty {
            Type::Pointer { .. } => true,
            Type::Array(element, _) => self.contains_pointer(element, span)?,
            Type::Record(_) => {
                let fields = self.resolve(ty, span)?.layout.fields;
                let mut found = false;
                for field in fields {
                    found |= self.contains_pointer(&field.ty, span)?;
                }
                found
            }
            _ => false,
        })
    }
}
