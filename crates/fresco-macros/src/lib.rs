use proc_macro::TokenStream;
use quote::quote;
use std::collections::BTreeMap;
use syn::{
    Block, Expr, Ident, LitStr, Result, Token,
    parse::{Parse, ParseStream},
    parse_macro_input,
};

struct BuiltinDef {
    name: LitStr,
    signature: SignatureType,
    check_body: Block,
    docs: Option<LitStr>,
}

struct TypeDeclDef {
    id: LitStr,
    kind: Ident,
    docs: LitStr,
}

struct EnumVariantDef {
    name: LitStr,
    docs: LitStr,
}

struct EnumDeclDef {
    id: LitStr,
    docs: LitStr,
    variants: Vec<EnumVariantDef>,
    contextual: bool,
}

enum SignatureType {
    Single {
        receiver: Option<Ident>,
        args: Vec<ArgDef>,
        result: Vec<Ident>,
        caps: Vec<Ident>,
    },
    Discriminated {
        discriminator: LitStr,
        default: Option<LitStr>,
        variants: BTreeMap<String, Vec<ArgDef>>,
        result: Vec<Ident>,
        caps: Vec<Ident>,
    },
}

#[derive(Clone)]
struct ArgDef {
    name: Ident,
    ty: ArgType,
    viz_role: Option<Ident>,
    doc: LitStr,
}

#[derive(Clone)]
enum ArgType {
    Required(Ident),
    Optional(Ident),
}

impl Parse for BuiltinDef {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        // name = "..."
        input.parse::<Ident>()?;
        input.parse::<Token![=]>()?;
        let name = input.parse::<LitStr>()?;
        input.parse::<Token![,]>()?;

        // signature = single { ... } or signature = discriminated { ... }
        input.parse::<Ident>()?;
        input.parse::<Token![=]>()?;
        let sig_type = input.parse::<Ident>()?;

        let signature = if sig_type == "single" {
            let sig_content;
            syn::braced!(sig_content in input);

            // Parse receiver if present
            let mut receiver = None;
            if sig_content.peek(Ident) && sig_content.peek2(Token![=]) {
                let lookahead = sig_content.fork();
                let key = lookahead.parse::<Ident>()?;
                if key == "receiver" {
                    sig_content.parse::<Ident>()?;
                    sig_content.parse::<Token![=]>()?;
                    receiver = Some(sig_content.parse::<Ident>()?);
                    sig_content.parse::<Token![,]>()?;
                }
            }

            // args(...)
            let args = parse_args(&sig_content)?;
            sig_content.parse::<Token![,]>()?;

            // result = Type
            sig_content.parse::<Ident>()?;
            sig_content.parse::<Token![=]>()?;
            let result = parse_result_types(&sig_content)?;
            sig_content.parse::<Token![,]>()?;

            // caps = CAP1 | CAP2 | ...
            let caps = parse_caps(&sig_content)?;
            let _ = sig_content.parse::<Token![,]>();

            SignatureType::Single {
                receiver,
                args,
                result,
                caps,
            }
        } else if sig_type == "discriminated" {
            let sig_content;
            syn::braced!(sig_content in input);

            // discriminator = "field_name"
            sig_content.parse::<Ident>()?;
            sig_content.parse::<Token![=]>()?;
            let discriminator = sig_content.parse::<LitStr>()?;
            sig_content.parse::<Token![,]>()?;

            // default = "variant" (optional)
            let mut default = None;
            if sig_content.peek(Ident) {
                let lookahead = sig_content.fork();
                let key = lookahead.parse::<Ident>()?;
                if key == "default" {
                    sig_content.parse::<Ident>()?;
                    sig_content.parse::<Token![=]>()?;
                    default = Some(sig_content.parse::<LitStr>()?);
                    sig_content.parse::<Token![,]>()?;
                }
            }

            // variants = { "name" => { args(...) }, ... }
            sig_content.parse::<Ident>()?;
            sig_content.parse::<Token![=]>()?;
            let variants_content;
            syn::braced!(variants_content in sig_content);

            let mut variants = BTreeMap::new();
            while !variants_content.is_empty() {
                let variant_name = variants_content.parse::<LitStr>()?;
                variants_content.parse::<Token![=>]>()?;

                let variant_sig;
                syn::braced!(variant_sig in variants_content);

                let args = parse_args(&variant_sig)?;
                let _ = variant_sig.parse::<Token![,]>();

                variants.insert(variant_name.value(), args);

                if !variants_content.is_empty() {
                    variants_content.parse::<Token![,]>()?;
                }
            }

            sig_content.parse::<Token![,]>()?;

            // result = Type
            sig_content.parse::<Ident>()?;
            sig_content.parse::<Token![=]>()?;
            let result = parse_result_types(&sig_content)?;
            sig_content.parse::<Token![,]>()?;

            // caps = CAP1 | CAP2 | ...
            let caps = parse_caps(&sig_content)?;
            let _ = sig_content.parse::<Token![,]>();

            SignatureType::Discriminated {
                discriminator,
                default,
                variants,
                result,
                caps,
            }
        } else {
            return Err(input.error("signature must be 'single' or 'discriminated'"));
        };

        input.parse::<Token![,]>()?;

        // check = |params...| { body }
        input.parse::<Ident>()?;
        input.parse::<Token![=]>()?;

        // Parse closure and extract body
        let closure = input.parse::<Expr>()?;
        let check_body = match closure {
            Expr::Closure(closure) => *closure.body,
            _ => return Err(input.error("expected closure")),
        };

        // Extract block from body
        let check_body = match check_body {
            Expr::Block(block) => block.block,
            other => {
                // Wrap single expression in a block
                Block {
                    brace_token: Default::default(),
                    stmts: vec![syn::Stmt::Expr(other, None)],
                }
            }
        };
        let mut docs = None;
        if input.peek(Ident) {
            let lookahead = input.fork();
            let key = lookahead.parse::<Ident>()?;
            if key == "docs" {
                input.parse::<Ident>()?;
                input.parse::<Token![=]>()?;
                docs = Some(input.parse::<LitStr>()?);
            }
        }

        let _ = input.parse::<Token![,]>().ok();
        let _ = input.parse::<Token![,]>();

        Ok(BuiltinDef {
            name,
            signature,
            docs,
            check_body,
        })
    }
}

impl Parse for TypeDeclDef {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let id = input.parse::<LitStr>()?;
        input.parse::<Token![,]>()?;
        let kind = input.parse::<Ident>()?;
        input.parse::<Token![,]>()?;
        let docs = input.parse::<LitStr>()?;
        let _ = input.parse::<Token![,]>();
        Ok(Self { id, kind, docs })
    }
}

impl Parse for EnumDeclDef {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let id = input.parse::<LitStr>()?;
        input.parse::<Token![,]>()?;
        let docs = input.parse::<LitStr>()?;
        input.parse::<Token![,]>()?;

        let variants_content;
        syn::bracketed!(variants_content in input);
        let mut variants = Vec::new();
        while !variants_content.is_empty() {
            let tuple;
            syn::parenthesized!(tuple in variants_content);
            let name = tuple.parse::<LitStr>()?;
            tuple.parse::<Token![,]>()?;
            let docs = tuple.parse::<LitStr>()?;
            variants.push(EnumVariantDef { name, docs });

            if !variants_content.is_empty() {
                variants_content.parse::<Token![,]>()?;
            }
        }

        let _ = input.parse::<Token![,]>();
        let contextual = if input.is_empty() {
            false
        } else {
            let mode = input.parse::<Ident>()?;
            if mode != "contextual" {
                return Err(syn::Error::new(mode.span(), "expected `contextual`"));
            }
            let _ = input.parse::<Token![,]>();
            true
        };
        Ok(Self {
            id,
            docs,
            variants,
            contextual,
        })
    }
}

fn parse_args(input: ParseStream<'_>) -> Result<Vec<ArgDef>> {
    input.parse::<Ident>()?; // "args"
    let args_content;
    syn::parenthesized!(args_content in input);

    let mut args = Vec::new();
    while !args_content.is_empty() {
        let arg_name = args_content.parse::<Ident>()?;
        args_content.parse::<Token![:]>()?;

        let ty = if args_content.peek(Ident) {
            let type_ident = args_content.parse::<Ident>()?;
            if type_ident == "Optional" {
                args_content.parse::<Token![<]>()?;
                let inner = args_content.parse::<Ident>()?;
                args_content.parse::<Token![>]>()?;
                ArgType::Optional(inner)
            } else {
                ArgType::Required(type_ident)
            }
        } else {
            return Err(args_content.error("expected type"));
        };

        let viz_role = if args_content.peek(Token![@]) {
            args_content.parse::<Token![@]>()?;
            Some(args_content.parse::<Ident>()?)
        } else {
            None
        };

        args_content.parse::<Token![=]>()?;
        let doc = args_content.parse::<LitStr>()?;

        args.push(ArgDef {
            name: arg_name,
            ty,
            viz_role,
            doc,
        });

        if !args_content.is_empty() {
            args_content.parse::<Token![,]>()?;
        }
    }

    Ok(args)
}

fn parse_caps(input: ParseStream<'_>) -> Result<Vec<Ident>> {
    input.parse::<Ident>()?; // "caps"
    input.parse::<Token![=]>()?;
    let mut caps = vec![input.parse::<Ident>()?];
    while input.peek(Token![|]) {
        input.parse::<Token![|]>()?;
        caps.push(input.parse::<Ident>()?);
    }
    Ok(caps)
}

fn is_numeric_value_doc(doc: &str) -> bool {
    doc.contains("scalar, vec2, vec3, or vec4")
        || doc.contains("value (scalar, vec2, vec3, or vec4)")
        || doc.contains("angle in radians (scalar, vec2, vec3, or vec4)")
}

fn synthesize_builtin_docs(
    name: &str,
    receiver: Option<&Ident>,
    args: &[ArgDef],
    result: &[Ident],
) -> String {
    let primary_result = result.first().map(ToString::to_string);

    if args.is_empty() {
        return match primary_result.as_deref() {
            Some("Shape") => format!("Create a {name} shape."),
            Some("Layer") => format!("Create a {name} layer."),
            _ => format!("{name} builtin."),
        };
    }

    let arg_docs = args.iter().map(|arg| arg.doc.value()).collect::<Vec<_>>();
    let arg_count = args.len();

    if arg_count == 1 && is_numeric_value_doc(&arg_docs[0]) {
        return format!("Apply {name} component-wise to scalar or vector inputs.");
    }

    if receiver.is_some() && arg_count == 1 {
        let arg_doc = &arg_docs[0];
        if arg_doc.contains("color or gradient") {
            return format!(
                "Apply {name} to a receiver using the provided color or gradient style."
            );
        }
        if arg_doc.contains("radius") {
            return format!("Apply {name} to a receiver using the provided radius.");
        }
        if arg_doc.contains("width") {
            return format!("Apply {name} to a receiver using the provided width.");
        }
        return format!("Apply {name} to a receiver using the provided argument.");
    }

    if matches!(primary_result.as_deref(), Some("Shape")) {
        return format!("Create a {name} shape from the provided arguments.");
    }

    if matches!(primary_result.as_deref(), Some("Layer")) {
        return format!("Create a {name} layer from the provided arguments.");
    }

    if arg_count >= 2 {
        if arg_docs.iter().any(|doc| doc.contains("first value"))
            && arg_docs.iter().any(|doc| doc.contains("second value"))
        {
            return format!("Combine two values with {name}.");
        }
        if arg_docs.iter().any(|doc| doc.contains("input coordinate")) {
            return format!("Generate a {name} value from the provided input coordinates.");
        }
    }

    format!("Use {name} with the provided arguments.")
}

fn parse_result_types(input: ParseStream<'_>) -> Result<Vec<Ident>> {
    let mut results = vec![input.parse::<Ident>()?];
    while input.peek(Token![|]) {
        input.parse::<Token![|]>()?;
        results.push(input.parse::<Ident>()?);
    }
    Ok(results)
}

#[proc_macro]
pub fn builtin(input: TokenStream) -> TokenStream {
    let def = parse_macro_input!(input as BuiltinDef);

    match def.signature {
        SignatureType::Single {
            receiver,
            args,
            result,
            caps,
        } => generate_single_builtin(
            &def.name,
            receiver,
            args,
            result,
            caps,
            def.check_body,
            def.docs.as_ref(),
        ),
        SignatureType::Discriminated {
            discriminator,
            default,
            variants,
            result,
            caps,
        } => generate_discriminated_builtin(DiscriminatedBuiltinInput {
            name: &def.name,
            discriminator,
            default,
            variants,
            result,
            caps,
            check_body: def.check_body,
            docs: def.docs.as_ref(),
        }),
    }
}

#[proc_macro]
pub fn type_decl(input: TokenStream) -> TokenStream {
    let def = parse_macro_input!(input as TypeDeclDef);
    let id = def.id;
    let kind = def.kind;
    let docs = def.docs;

    TokenStream::from(quote! {
        inventory::submit! {
            crate::registry::TypeDecl {
                id: crate::registry::TypeId(#id),
                kind: crate::registry::PrimitiveType::#kind,
                docs: #docs,
            }
        }
    })
}

#[proc_macro]
pub fn enum_decl(input: TokenStream) -> TokenStream {
    let def = parse_macro_input!(input as EnumDeclDef);
    let id = def.id;
    let docs = def.docs;
    let contextual = def.contextual;
    let variant_names: Vec<_> = def.variants.iter().map(|v| &v.name).collect();
    let variant_docs: Vec<_> = def.variants.iter().map(|v| &v.docs).collect();

    TokenStream::from(quote! {
        inventory::submit! {
            crate::registry::EnumDecl {
                id: crate::registry::EnumId(#id),
                docs: #docs,
                contextual: #contextual,
                variants: &[
                    #(
                        crate::registry::EnumVariantDecl {
                            name: #variant_names,
                            docs: #variant_docs,
                        }
                    ),*
                ],
            }
        }
    })
}

fn generate_single_builtin(
    name: &LitStr,
    receiver: Option<Ident>,
    args: Vec<ArgDef>,
    result: Vec<Ident>,
    caps: Vec<Ident>,
    check_body: Block,
    docs: Option<&LitStr>,
) -> TokenStream {
    let name_str = name;
    let func_name = if let Some(recv) = &receiver {
        Ident::new(
            &format!(
                "{}_{}_check_impl",
                builtin_rust_name(&name.value()),
                recv.to_string().to_lowercase()
            ),
            name.span(),
        )
    } else {
        Ident::new(
            &format!("{}_check_impl", builtin_rust_name(&name.value())),
            name.span(),
        )
    };

    // Build function parameters with explicit types
    let mut func_params = vec![quote! { ctx: &mut crate::check::Checker }];

    // Add receiver parameter if present
    let (_recv_param, recv_type_ref) = if let Some(recv) = &receiver {
        let recv_param_name = Ident::new("_receiver", recv.span());
        let recv_id_type = Ident::new(&format!("{}Id", recv), recv.span());
        func_params.push(quote! { #recv_param_name: crate::hir::#recv_id_type });

        let prim_type = match recv.to_string().as_str() {
            "Shape" => quote! { crate::registry::PrimitiveType::Shape },
            "Layer" => quote! { crate::registry::PrimitiveType::Layer },
            _ => panic!("Unknown receiver type: {}", recv),
        };

        (
            Some(recv_param_name),
            quote! { Some(crate::registry::TypeRef::Primitive(#prim_type)) },
        )
    } else {
        (None, quote! { None })
    };

    func_params.push(quote! { bag: &mut crate::check::ArgBag });

    // Build argument extraction code and declarations
    let mut extract_stmts = Vec::new();
    let mut arg_decls = Vec::new();

    for arg in &args {
        let arg_name = &arg.name;
        let arg_doc = &arg.doc;
        let arg_viz_role = arg.viz_role.as_ref();

        match &arg.ty {
            ArgType::Required(ty) => {
                let prim_type = type_to_primitive(ty);

                if ty == "Expr" || ty == "ColorExpr" {
                    extract_stmts.push(quote! {
                        let #arg_name = bag.require(stringify!(#arg_name), &mut ctx.diags)?;
                    });
                } else {
                    let converter = type_converter(ty);
                    extract_stmts.push(quote! {
                        let __arg = bag.require(stringify!(#arg_name), &mut ctx.diags)?;
                        let #arg_name = ctx.#converter(__arg)?;
                    });
                }

                if let Some(viz_role) = arg_viz_role {
                    arg_decls.push(quote! {
                        crate::registry::BuiltinArgDecl::required_with_role(
                            stringify!(#arg_name),
                            crate::registry::TypeRef::Primitive(#prim_type),
                            stringify!(#viz_role),
                            #arg_doc,
                        )
                    });
                } else {
                    arg_decls.push(quote! {
                        crate::registry::BuiltinArgDecl::required(
                            stringify!(#arg_name),
                            crate::registry::TypeRef::Primitive(#prim_type),
                            #arg_doc,
                        )
                    });
                }
            }
            ArgType::Optional(ty) => {
                let prim_type = type_to_primitive(ty);

                if ty == "Expr" || ty == "ColorExpr" {
                    extract_stmts.push(quote! {
                        let #arg_name = bag.take(stringify!(#arg_name));
                    });
                } else {
                    let converter = type_converter(ty);
                    extract_stmts.push(quote! {
                        let #arg_name = bag.take(stringify!(#arg_name))
                            .and_then(|__arg| ctx.#converter(__arg));
                    });
                }

                if let Some(viz_role) = arg_viz_role {
                    arg_decls.push(quote! {
                        crate::registry::BuiltinArgDecl::optional_with_role(
                            stringify!(#arg_name),
                            crate::registry::TypeRef::Primitive(#prim_type),
                            stringify!(#viz_role),
                            #arg_doc,
                        )
                    });
                } else {
                    arg_decls.push(quote! {
                        crate::registry::BuiltinArgDecl::optional(
                            stringify!(#arg_name),
                            crate::registry::TypeRef::Primitive(#prim_type),
                            #arg_doc,
                        )
                    });
                }
            }
        }
    }

    // Build result type references (first is primary, rest are alternatives)
    let result_type_ref = type_to_typeref(&result[0]);
    let result_alt_type_refs: Vec<_> = result.iter().skip(1).map(type_to_typeref).collect();

    // Build caps bits expression
    let caps_bits_expr = build_caps_bits_expr(&caps);

    // Determine output cap from result type
    let out_cap = result_to_out_cap(&result[0]);

    // Build lowering
    let lowering = if let Some(recv) = &receiver {
        let lowering_fn = match recv.to_string().as_str() {
            "Shape" => quote! { shape },
            "Layer" => quote! { layer },
            _ => panic!("Unknown receiver type for lowering: {}", recv),
        };
        quote! {
            crate::registry::BuiltinLowering::ImplReceiver(
                crate::registry::BuiltinImplReceiverFn::#lowering_fn(#func_name)
            )
        }
    } else {
        quote! { crate::registry::BuiltinLowering::Impl(#func_name) }
    };

    let docs = docs.map(|docs| quote! { #docs }).unwrap_or_else(|| {
        let synthesized = synthesize_builtin_docs(&name.value(), receiver.as_ref(), &args, &result);
        let lit = LitStr::new(&synthesized, name.span());
        quote! { #lit }
    });

    let expanded = quote! {
        pub fn #func_name(#(#func_params),*) -> Option<crate::check::Value> {
            #(#extract_stmts)*

            Some(#check_body)
        }

        #[doc(hidden)]
        inventory::submit! {
            crate::registry::BuiltinDecl {
                id: crate::registry::BuiltinId(#name_str),
                name: #name_str,
                discriminator: None,
                signature: crate::registry::BuiltinSignature {
                    receiver: #recv_type_ref,
                    args: &[#(#arg_decls),*],
                    result: #result_type_ref,
                    result_alternatives: &[#(#result_alt_type_refs),*],
                    caps: crate::builtin_catalog::BuiltinCaps::from_bits_retain(
                        #out_cap.bits() | #caps_bits_expr
                    ),
                },
                lowering: #lowering,
                docs: #docs,
            }
        }
    };

    // Keep generated checker symbols local: camelCase and snake_case aliases
    // can normalize to the same Rust identifier while registering distinct names.
    TokenStream::from(quote! { const _: () = { #expanded }; })
}

struct DiscriminatedBuiltinInput<'a> {
    name: &'a LitStr,
    discriminator: LitStr,
    default: Option<LitStr>,
    variants: BTreeMap<String, Vec<ArgDef>>,
    result: Vec<Ident>,
    caps: Vec<Ident>,
    check_body: Block,
    docs: Option<&'a LitStr>,
}

fn generate_discriminated_builtin(input: DiscriminatedBuiltinInput<'_>) -> TokenStream {
    let DiscriminatedBuiltinInput {
        name,
        discriminator,
        default,
        variants,
        result,
        caps,
        check_body,
        docs,
    } = input;

    let name_str = name;
    let func_name = Ident::new(
        &format!("{}_check_impl", builtin_rust_name(&name.value())),
        name.span(),
    );
    let discriminator_str = &discriminator;
    let default_str = default.as_ref();
    let mut variant_names_sorted = variants.keys().cloned().collect::<Vec<_>>();
    variant_names_sorted.sort();
    let variant_name_lits = variant_names_sorted
        .iter()
        .map(|name| LitStr::new(name, proc_macro2::Span::call_site()))
        .collect::<Vec<_>>();

    // For discriminated signatures, the check function receives:
    // ctx, variant name (as &str), and the full ArgBag
    let func_params = vec![
        quote! { ctx: &mut crate::check::Checker },
        quote! { variant: &str },
        quote! { args: &mut crate::check::ArgBag },
    ];

    // Ordered maps keep emitted signatures and documentation stable across compiler builds.
    // Collect all arguments from all variants for the signature registration
    let mut all_args_map: BTreeMap<String, ArgDef> = BTreeMap::new();
    for variant_args in variants.values() {
        for arg in variant_args {
            let key = arg.name.to_string();
            // If we see the same arg in multiple variants, keep the first one
            // (in practice, discriminated signatures usually have non-overlapping args per variant)
            all_args_map.entry(key).or_insert_with(|| ArgDef {
                name: arg.name.clone(),
                ty: arg.ty.clone(),
                viz_role: arg.viz_role.clone(),
                doc: arg.doc.clone(),
            });
        }
    }

    let mut arg_decls = Vec::new();
    for arg in all_args_map.values() {
        let arg_name = &arg.name;
        let arg_doc = &arg.doc;
        let arg_viz_role = arg.viz_role.as_ref();

        match &arg.ty {
            ArgType::Required(ty) => {
                let prim_type = type_to_primitive(ty);
                // Discriminated builtins validate required fields per variant in the
                // generated impl function. The coarse registry signature must stay
                // permissive enough for dispatch to reach that variant logic.
                if let Some(viz_role) = arg_viz_role {
                    arg_decls.push(quote! {
                        crate::registry::BuiltinArgDecl::optional_with_role(
                            stringify!(#arg_name),
                            crate::registry::TypeRef::Primitive(#prim_type),
                            stringify!(#viz_role),
                            #arg_doc,
                        )
                    });
                } else {
                    arg_decls.push(quote! {
                        crate::registry::BuiltinArgDecl::optional(
                            stringify!(#arg_name),
                            crate::registry::TypeRef::Primitive(#prim_type),
                            #arg_doc,
                        )
                    });
                }
            }
            ArgType::Optional(ty) => {
                let prim_type = type_to_primitive(ty);
                if let Some(viz_role) = arg_viz_role {
                    arg_decls.push(quote! {
                        crate::registry::BuiltinArgDecl::optional_with_role(
                            stringify!(#arg_name),
                            crate::registry::TypeRef::Primitive(#prim_type),
                            stringify!(#viz_role),
                            #arg_doc,
                        )
                    });
                } else {
                    arg_decls.push(quote! {
                        crate::registry::BuiltinArgDecl::optional(
                            stringify!(#arg_name),
                            crate::registry::TypeRef::Primitive(#prim_type),
                            #arg_doc,
                        )
                    });
                }
            }
        }
    }

    // Build result type references (first is primary, rest are alternatives)
    let result_type_ref = type_to_typeref(&result[0]);
    let result_alt_type_refs: Vec<_> = result.iter().skip(1).map(type_to_typeref).collect();

    // Build caps bits expression
    let caps_bits_expr = build_caps_bits_expr(&caps);

    // Determine output cap from result type
    let out_cap = result_to_out_cap(&result[0]);

    // Generate default option
    let default_option = if let Some(d) = default_str {
        quote! { Some(#d) }
    } else {
        quote! { None }
    };

    let docs = docs.map(|docs| quote! { #docs }).unwrap_or_else(|| {
        let flat_args = all_args_map.values().cloned().collect::<Vec<_>>();
        let synthesized = synthesize_builtin_docs(&name.value(), None, &flat_args, &result);
        let lit = LitStr::new(&synthesized, name.span());
        quote! { #lit }
    });

    let expanded = quote! {
        pub fn #func_name(#(#func_params),*) -> Option<crate::check::Value> {
            Some(#check_body)
        }

        #[doc(hidden)]
        inventory::submit! {
            crate::registry::BuiltinDecl {
                id: crate::registry::BuiltinId(#name_str),
                name: #name_str,
                discriminator: Some(crate::registry::BuiltinDiscriminatorDecl {
                    name: #discriminator_str,
                    default: #default_option,
                    variants: &[#(#variant_name_lits),*],
                }),
                signature: crate::registry::BuiltinSignature {
                    receiver: None,
                    args: &[#(#arg_decls),*],
                    result: #result_type_ref,
                    result_alternatives: &[#(#result_alt_type_refs),*],
                    caps: crate::builtin_catalog::BuiltinCaps::from_bits_retain(
                        #out_cap.bits() | #caps_bits_expr
                    ),
                },
                lowering: crate::registry::BuiltinLowering::ImplDiscriminated {
                    discriminator: #discriminator_str,
                    default: #default_option,
                    impl_fn: #func_name,
                },
                docs: #docs,
            }
        }
    };

    // Keep generated checker symbols local: camelCase and snake_case aliases
    // can normalize to the same Rust identifier while registering distinct names.
    TokenStream::from(quote! { const _: () = { #expanded }; })
}

fn type_converter(ty: &Ident) -> proc_macro2::TokenStream {
    match ty.to_string().as_str() {
        "Vec2" => quote! { as_vec2 },
        "Vec3" => quote! { as_vec3 },
        "Vec4" => quote! { as_vec4 },
        "Scalar" => quote! { as_scalar },
        "Color" => quote! { as_color_expr },
        _ => panic!("Unknown type: {}", ty),
    }
}

fn type_to_primitive(ty: &Ident) -> proc_macro2::TokenStream {
    match ty.to_string().as_str() {
        "Vec2" => quote! { crate::registry::PrimitiveType::Vec2 },
        "Vec3" => quote! { crate::registry::PrimitiveType::Vec3 },
        "Vec4" => quote! { crate::registry::PrimitiveType::Vec4 },
        "Scalar" => quote! { crate::registry::PrimitiveType::Scalar },
        "Color" => quote! { crate::registry::PrimitiveType::Color },
        "Expr" => quote! { crate::registry::PrimitiveType::Expr },
        "ColorExpr" => quote! { crate::registry::PrimitiveType::Color },
        _ => panic!("Unknown type: {}", ty),
    }
}

fn type_to_typeref(ty: &Ident) -> proc_macro2::TokenStream {
    match ty.to_string().as_str() {
        "Vec2" => {
            quote! { crate::registry::TypeRef::Primitive(crate::registry::PrimitiveType::Vec2) }
        }
        "Vec3" => {
            quote! { crate::registry::TypeRef::Primitive(crate::registry::PrimitiveType::Vec3) }
        }
        "Vec4" => {
            quote! { crate::registry::TypeRef::Primitive(crate::registry::PrimitiveType::Vec4) }
        }
        "Scalar" => {
            quote! { crate::registry::TypeRef::Primitive(crate::registry::PrimitiveType::Scalar) }
        }
        "Mask" => {
            quote! { crate::registry::TypeRef::Primitive(crate::registry::PrimitiveType::Mask) }
        }
        "Shape" => {
            quote! { crate::registry::TypeRef::Primitive(crate::registry::PrimitiveType::Shape) }
        }
        "Layer" => {
            quote! { crate::registry::TypeRef::Primitive(crate::registry::PrimitiveType::Layer) }
        }
        "Color" => {
            quote! { crate::registry::TypeRef::Primitive(crate::registry::PrimitiveType::Color) }
        }
        "ColorField" => {
            quote! { crate::registry::TypeRef::Primitive(crate::registry::PrimitiveType::ColorField) }
        }
        "Gradient" => {
            quote! { crate::registry::TypeRef::Primitive(crate::registry::PrimitiveType::Gradient) }
        }
        _ => panic!("Unknown result type: {}", ty),
    }
}

fn result_to_out_cap(ty: &Ident) -> proc_macro2::TokenStream {
    match ty.to_string().as_str() {
        "Vec2" => quote! { crate::builtin_catalog::BuiltinCaps::VEC2_OUT },
        "Vec3" => quote! { crate::builtin_catalog::BuiltinCaps::VEC3_OUT },
        "Vec4" => quote! { crate::builtin_catalog::BuiltinCaps::VEC4_OUT },
        "Scalar" => quote! { crate::builtin_catalog::BuiltinCaps::SCALAR_OUT },
        "Mask" => quote! { crate::builtin_catalog::BuiltinCaps::MASK_OUT },
        "Shape" => quote! { crate::builtin_catalog::BuiltinCaps::SHAPE_OUT },
        "Layer" => quote! { crate::builtin_catalog::BuiltinCaps::LAYER_OUT },
        "Color" => quote! { crate::builtin_catalog::BuiltinCaps::COLOR_OUT },
        "ColorField" => quote! { crate::builtin_catalog::BuiltinCaps::COLOR_OUT },
        "Gradient" => quote! { crate::builtin_catalog::BuiltinCaps::GRADIENT_OUT },
        _ => panic!("Unknown result type for out_cap: {}", ty),
    }
}

fn build_caps_bits_expr(caps: &[Ident]) -> proc_macro2::TokenStream {
    if caps.len() == 1 {
        let cap = &caps[0];
        quote! { crate::builtin_catalog::BuiltinCaps::#cap.bits() }
    } else {
        let cap_exprs = caps
            .iter()
            .map(|c| quote! { crate::builtin_catalog::BuiltinCaps::#c.bits() });
        quote! { #( #cap_exprs )|* }
    }
}

// WGSL names may be camelCase; generated Rust function names must be snake_case.
fn builtin_rust_name(name: &str) -> String {
    let mut result = String::new();
    for ch in name.chars() {
        if ch.is_ascii_uppercase() {
            if !result.is_empty() {
                result.push('_');
            }
            result.push(ch.to_ascii_lowercase());
        } else {
            result.push(ch);
        }
    }
    result
}
