use std::collections::HashSet;

use proc_macro::Delimiter;
use virtue::generate::FnSelfArg;
use virtue::parse::{Attribute, AttributeLocation, EnumBody, StructBody};
use virtue::prelude::*;

#[proc_macro_derive(SSHEncode, attributes(sshwire))]
pub fn derive_encode(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let r = encode_inner(input).unwrap_or_else(|e| e.into_token_stream());
    r
}

#[proc_macro_derive(SSHDecode, attributes(sshwire))]
pub fn derive_decode(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    decode_inner(input).unwrap_or_else(|e| e.into_token_stream())
}

fn encode_inner(input: TokenStream) -> Result<TokenStream> {
    let parse = Parse::new(input)?;
    let (mut gen, att, body) = parse.into_generator();
    // println!("att {att:#?}");
    match body {
        Body::Struct(body) => {
            encode_struct(&mut gen, body)?;
        }
        Body::Enum(body) => {
            encode_enum(&mut gen, &att, body)?;
        }
    }
    gen.export_to_file("SSHEncode");
    gen.finish()
}

fn decode_inner(input: TokenStream) -> Result<TokenStream> {
    let parse = Parse::new(input)?;
    let (mut gen, att, body) = parse.into_generator();
    // println!("att {att:#?}");
    match body {
        Body::Struct(body) => {
            decode_struct(&mut gen, body)?;
        }
        Body::Enum(body) => {
            decode_enum(&mut gen, &att, body)?;
        }
    }
    gen.export_to_file("SSHDecode");
    gen.finish()
}

#[derive(Debug)]
enum ContainerAtt {
    /// The string of the method is prefixed to this enum.
    /// `#[sshwire(variant_prefix)]`
    VariantPrefix,

    /// Don't generate SSHEncodeEnum. Can't be used with SSHDecode derive.
    /// `#[sshwire(no_variant_names)]`
    NoNames,
}

#[derive(Debug)]
enum FieldAtt {
    /// A variant method will be encoded/decoded before the next field.
    /// eg `#[sshwire(variant_name = ch)]` for `ChannelRequest`
    VariantName(Ident),
    /// Any unknown variant name should be recorded here.
    /// This variant can't be written out.
    /// `#[sshwire(unknown))]`
    CaptureUnknown,
    /// The name of a variant, used by the parent struct
    /// `#[sshwire(variant = "exit-signal"))]`
    /// or
    /// `#[sshwire(variant = SSH_NAME_IDENT))]`
    Variant(TokenTree),
}

fn take_cont_atts(atts: &[Attribute]) -> Result<Vec<ContainerAtt>> {
    atts.iter()
        .filter_map(|a| {
            match a.location {
                AttributeLocation::Container => {
                    let mut s = a.tokens.stream().into_iter();
                    if &s.next().expect("missing attribute name").to_string()
                        != "sshwire"
                    {
                        // skip attributes other than "sshwire"
                        return None;
                    }
                    Some(if let Some(TokenTree::Group(g)) = s.next() {
                        let mut g = g.stream().into_iter();
                        let f = match g.next() {
                            Some(TokenTree::Ident(l))
                                if l.to_string() == "no_variant_names" =>
                            {
                                Ok(ContainerAtt::NoNames)
                            }

                            Some(TokenTree::Ident(l))
                                if l.to_string() == "variant_prefix" =>
                            {
                                Ok(ContainerAtt::VariantPrefix)
                            }

                            _ => Err(Error::Custom {
                                error: "Unknown sshwire atttribute".into(),
                                span: Some(a.tokens.span()),
                            }),
                        };

                        if let Some(_) = g.next() {
                            Err(Error::Custom {
                                error: "Extra unhandled parts".into(),
                                span: Some(a.tokens.span()),
                            })
                        } else {
                            f
                        }
                    } else {
                        Err(Error::Custom {
                            error: "#[sshwire(...)] attribute is missing (...) part"
                                .into(),
                            span: Some(a.tokens.span()),
                        })
                    })
                }
                _ => panic!("Non-field attribute for field: {a:#?}"),
            }
        })
        .collect()
}

// TODO: we could use virtue parse_tagged_attribute() though it doesn't support Literals
fn take_field_atts(atts: &[Attribute]) -> Result<Vec<FieldAtt>> {
    atts.iter()
        .filter_map(|a| {
            match a.location {
                AttributeLocation::Field | AttributeLocation::Variant => {
                    let mut s = a.tokens.stream().into_iter();
                    if &s.next().expect("missing attribute name").to_string()
                        != "sshwire"
                    {
                        // skip attributes other than "sshwire"
                        return None;
                    }
                    Some(if let Some(TokenTree::Group(g)) = s.next() {
                        let mut g = g.stream().into_iter();
                        let f = match g.next() {
                            Some(TokenTree::Ident(l))
                                if l.to_string() == "variant_name" =>
                            {
                                // check for '='
                                match g.next() {
                                    Some(TokenTree::Punct(p)) if p == '=' => (),
                                    _ => {
                                        return Some(Err(Error::Custom {
                                            error: "Missing '='".into(),
                                            span: Some(a.tokens.span()),
                                        }))
                                    }
                                }
                                match g.next() {
                                    Some(TokenTree::Ident(i)) => {
                                        Ok(FieldAtt::VariantName(i))
                                    }
                                    _ => Err(Error::ExpectedIdent(a.tokens.span())),
                                }
                            }

                            Some(TokenTree::Ident(l))
                                if l.to_string() == "unknown" =>
                            {
                                Ok(FieldAtt::CaptureUnknown)
                            }

                            Some(TokenTree::Ident(l))
                                if l.to_string() == "variant" =>
                            {
                                // check for '='
                                match g.next() {
                                    Some(TokenTree::Punct(p)) if p == '=' => (),
                                    _ => {
                                        return Some(Err(Error::Custom {
                                            error: "Missing '='".into(),
                                            span: Some(a.tokens.span()),
                                        }))
                                    }
                                }
                                if let Some(t) = g.next() {
                                    Ok(FieldAtt::Variant(t))
                                } else {
                                    Err(Error::Custom {
                                        error: "Missing expression".into(),
                                        span: Some(a.tokens.span()),
                                    })
                                }
                            }

                            _ => Err(Error::Custom {
                                error: "Unknown sshwire atttribute".into(),
                                span: Some(a.tokens.span()),
                            }),
                        };

                        if let Some(_) = g.next() {
                            Err(Error::Custom {
                                error: "Extra unhandled parts".into(),
                                span: Some(a.tokens.span()),
                            })
                        } else {
                            f
                        }
                    } else {
                        Err(Error::Custom {
                            error: "#[sshwire(...)] attribute is missing (...) part"
                                .into(),
                            span: Some(a.tokens.span()),
                        })
                    })
                }
                _ => panic!("Non-field attribute for field: {a:#?}"),
            }
        })
        .collect()
}

fn encode_struct(gen: &mut Generator, body: StructBody) -> Result<()> {
    gen.impl_for("crate::sshwire::SSHEncode")
        .generate_fn("enc")
        .with_generic_deps("E", ["crate::sshwire::SSHSink"])
        .with_self_arg(FnSelfArg::RefSelf)
        .with_arg("s", "&mut E")
        .with_return_type("Result<()>")
        .body(|fn_body| {
            match &body.fields {
                Fields::Tuple(v) => {
                    for (fname, f) in v.iter().enumerate() {
                        // we're only using single elements for newtype, don't bother with atts for now
                        if !f.attributes.is_empty() {
                            return Err(Error::Custom { error: "Attributes aren't allowed for tuple structs".into(), span: Some(f.span()) })
                        }
                        fn_body.push_parsed(format!("crate::sshwire::SSHEncode::enc(&self.{fname}, s)?;"))?;
                    }
                }
                Fields::Struct(v) => {
                    for f in v {
                        let fname = &f.0;
                        let atts = take_field_atts(&f.1.attributes)?;
                        for a in atts {
                            if let FieldAtt::VariantName(enum_field) = a {
                                // encode an enum field's variant name before this field
                                fn_body.push_parsed(format!("crate::sshwire::SSHEncode::enc(&self.{enum_field}.variant_name()?, s)?;"))?;
                            }
                        }
                        // println!("atts for {fname}: {atts:?}");
                        // TODO handle attributes
                        fn_body.push_parsed(format!("crate::sshwire::SSHEncode::enc(&self.{fname}, s)?;"))?;
                    }

                }
                _ => {
                    // empty
                }

            }
            fn_body.push_parsed("Ok(())")?;
            Ok(())
        })?;
    Ok(())
}

fn encode_enum(
    gen: &mut Generator,
    atts: &[Attribute],
    body: EnumBody,
) -> Result<()> {
    // if body.variants.is_empty() {
    //     return Ok(())
    // }

    let cont_atts = take_cont_atts(atts)?;

    gen.impl_for("crate::sshwire::SSHEncode")
        .generate_fn("enc")
        .with_generic_deps("S", ["crate::sshwire::SSHSink"])
        .with_self_arg(FnSelfArg::RefSelf)
        .with_arg("s", "&mut S")
        .with_return_type("Result<()>")
        .body(|fn_body| {
            if cont_atts.iter().any(|c| matches!(c, ContainerAtt::VariantPrefix)) {
                fn_body.push_parsed("crate::sshwire::SSHEncode::enc(&self.variant_name()?, s)?;")?;
            }

            fn_body.ident_str("match");
            fn_body.puncts("*");
            fn_body.ident_str("self");
            fn_body.group(Delimiter::Brace, |match_arm| {
                for var in &body.variants {
                    match_arm.ident_str("Self");
                    match_arm.puncts("::");
                    match_arm.ident(var.name.clone());

                    let atts = take_field_atts(&var.attributes)?;

                    let mut rhs = StreamBuilder::new();
                    match var.fields {
                        Fields::Unit => {
                            // nothing to do
                        }
                        Fields::Tuple(ref f) if f.len() == 1 => {
                            match_arm.group(Delimiter::Parenthesis, |item| {
                                item.ident_str("ref");
                                item.ident_str("i");
                                Ok(())
                            })?;
                            if atts.iter().any(|a| matches!(a, FieldAtt::CaptureUnknown)) {
                                rhs.push_parsed("return Error::bug_msg(\"Can't encode Unknown\")")?;
                            } else {
                                rhs.push_parsed(format!("crate::sshwire::SSHEncode::enc(i, s)?;"))?;
                            }

                        }
                        _ => return Err(Error::Custom { error: "SSHEncode currently only implements Unit or single value enum variants. ".into(), span: None})
                    }

                    match_arm.puncts("=>");
                    match_arm.group(Delimiter::Brace, |var_body| {
                        var_body.append(rhs);
                        Ok(())
                    })?;
                }
                Ok(())
            })?;
            fn_body.push_parsed("Ok(())")?;
            Ok(())
        })?;

    if !cont_atts.iter().any(|c| matches!(c, ContainerAtt::NoNames)) {
        encode_enum_names(gen, atts, body)?;
    }
    Ok(())
}

fn field_att_var_names(name: &Ident, mut atts: Vec<FieldAtt>) -> Result<TokenTree> {
    let mut v = vec![];
    while let Some(p) = atts.pop() {
        if let FieldAtt::Variant(t) = p {
            v.push(t);
        }
    }
    if v.len() != 1 {
        return Err(Error::Custom { error: format!("One #[sshwire(variant = ...)] attribute is required for each enum field, missing for {:?}", name), span: None});
    }
    Ok(v.pop().unwrap())
}

fn encode_enum_names(
    gen: &mut Generator,
    _atts: &[Attribute],
    body: EnumBody,
) -> Result<()> {
    gen.impl_for("crate::sshwire::SSHEncodeEnum")
        .generate_fn("variant_name")
        .with_self_arg(FnSelfArg::RefSelf)
        .with_return_type("Result<&'static str>")
        .body(|fn_body| {
            fn_body.push_parsed("let r = match self")?;
            fn_body.group(Delimiter::Brace, |match_arm| {
                for var in &body.variants {
                    match_arm.ident_str("Self");
                    match_arm.puncts("::");
                    match_arm.ident(var.name.clone());

                    let mut rhs = StreamBuilder::new();
                    let atts = take_field_atts(&var.attributes)?;
                    if atts.iter().any(|a| matches!(a, FieldAtt::CaptureUnknown)) {
                        rhs.push_parsed("return Error::bug_msg(\"Can't encode Unknown\")")?;
                    } else {
                        rhs.push(field_att_var_names(&var.name, atts)?);
                    }

                    match var.fields {
                        Fields::Unit => {
                            // nothing to do
                        }
                        Fields::Tuple(ref f) if f.len() == 1 => {
                            match_arm.group(Delimiter::Parenthesis, |item| {
                                item.ident_str("_");
                                Ok(())
                            })?;

                        }
                        _ => return Err(Error::Custom { error: "SSHEncode currently only implements Unit or single value enum variants. ".into(), span: None})
                    }

                    match_arm.puncts("=>");
                    match_arm.group(Delimiter::Brace, |var_body| {
                        var_body.append(rhs);
                        Ok(())
                    })?;
                }
                Ok(())
            })?;
            fn_body.push_parsed("; Ok(r)")?;

            Ok(())
        })?;

    Ok(())
}

fn decode_struct(gen: &mut Generator, body: StructBody) -> Result<()> {
    gen.impl_for_with_lifetimes("crate::sshwire::SSHDecode", ["de"])
        .generate_fn("dec")
        .with_generic_deps("S", ["crate::sshwire::SSHSource<'de>"])
        .with_arg("s", "&mut S")
        .with_return_type(format!("Result<Self>"))
        .body(|fn_body| {
            let mut named_enums = HashSet::new();
            if let Fields::Struct(v) = &body.fields {
                for f in v {
                    let atts = take_field_atts(&f.1.attributes)?;
                    for a in atts {
                        if let FieldAtt::VariantName(enum_field) = a {
                            named_enums.insert(enum_field.to_string());
                            fn_body.push_parsed(format!("let enum_name_{enum_field} = crate::sshwire::SSHDecode::dec(s)?;"))?;
                        }
                    }
                    let fname = &f.0;
                    if named_enums.contains(&fname.to_string()) {
                        fn_body.push_parsed(format!("let field_{fname} =  crate::sshwire::SSHDecodeEnum::dec_enum(s, enum_name_{fname})?;"))?;
                    } else {
                        fn_body.push_parsed(format!("let field_{fname} = crate::sshwire::SSHDecode::dec(s)?;"))?;
                    }
                }
            }
            fn_body.ident_str("Ok");
            fn_body.group(Delimiter::Parenthesis, |fn_body| {
                match &body.fields {
                    Fields::Tuple(_) => {
                        // we don't handle attributes for Tuple Structs - only use as newtype
                        fn_body.ident_str("Self");
                        fn_body.group(Delimiter::Parenthesis, |args| {
                            for _ in body.fields.names() {
                                args.push_parsed(format!("crate::sshwire::SSHDecode::dec(s)?,"))?;
                            }
                            Ok(())
                        })?;
                    }
                    Fields::Struct(v) => {
                        fn_body.ident_str("Self");
                        fn_body.group(Delimiter::Brace, |args| {
                            for f in v {
                                let fname = &f.0;
                                args.push_parsed(format!("{fname}: field_{fname},"))?;
                            }
                            Ok(())
                        })?;
                    }
                    _ => {
                        // empty
                    }

                }
                Ok(())
            })?;
            Ok(())
        })?;
    Ok(())
}

fn decode_enum(
    gen: &mut Generator,
    atts: &[Attribute],
    body: EnumBody,
) -> Result<()> {
    let cont_atts = take_cont_atts(atts)?;

    if cont_atts.iter().any(|c| matches!(c, ContainerAtt::NoNames)) {
        return Err(Error::Custom {
            error:
                "SSHDecode derive can't be used with #[sshwire(no_variant_names)]"
                    .into(),
            span: None,
        });
    }

    // SSHDecode trait if it is self describing
    if cont_atts.iter().any(|c| matches!(c, ContainerAtt::VariantPrefix)) {
        decode_enum_variant_prefix(gen, atts, &body)?;
    }

    decode_enum_names(gen, atts, &body)?;
    Ok(())
}

fn decode_enum_variant_prefix(
    gen: &mut Generator,
    _atts: &[Attribute],
    _body: &EnumBody,
) -> Result<()> {
    gen.impl_for_with_lifetimes("crate::sshwire::SSHDecode", ["de"])
        .generate_fn("dec")
        .with_generic_deps("S", ["crate::sshwire::SSHSource<'de>"])
        .with_arg("s", "&mut S")
        .with_return_type(format!("Result<Self>"))
        .body(|fn_body| {
            fn_body
                .push_parsed("let variant = crate::sshwire::SSHDecode::dec(s)?;")?;
            fn_body.push_parsed(
                "crate::sshwire::SSHDecodeEnum::dec_enum(s, variant)",
            )?;
            Ok(())
        })
}

fn decode_enum_names(
    gen: &mut Generator,
    _atts: &[Attribute],
    body: &EnumBody,
) -> Result<()> {
    gen.impl_for_with_lifetimes("crate::sshwire::SSHDecodeEnum", ["de"])
        .generate_fn("dec_enum")
        .with_generic_deps("S", ["crate::sshwire::SSHSource<'de>"])
        .with_arg("s", "&mut S")
        .with_arg("variant", "&'de str")
        .with_return_type(format!("Result<Self>"))
        .body(|fn_body| {
            fn_body.push_parsed("let r = match variant")?;
            fn_body.group(Delimiter::Brace, |match_arm| {
                let mut unknown_arm = None;
                for var in &body.variants {
                    let atts = take_field_atts(&var.attributes)?;
                    if atts.iter().any(|a| matches!(a, FieldAtt::CaptureUnknown)) {
                        // create the Unknown fallthrough but it will be at the end of the match list
                        let mut m = StreamBuilder::new();
                        m.push_parsed(format!("unk => Self::{}(Unknown(unk))", var.name))?;
                        if unknown_arm.replace(m).is_some() {
                            return Err(Error::Custom { error: "only one variant can have #[sshwire(unknown)]".into(), span: None})
                        }
                    } else {
                        let var_name = field_att_var_names(&var.name, atts)?;
                        match_arm.push_parsed(format!("{} => ", var_name))?;
                        match_arm.group(Delimiter::Brace, |var_body| {
                            match var.fields {
                                Fields::Unit => {
                                    var_body.push_parsed(format!("Self::{}", var.name))?;
                                }
                                Fields::Tuple(ref f) if f.len() == 1 => {
                                    var_body.push_parsed(format!("Self::{}(crate::sshwire::SSHDecode::dec(s)?)", var.name))?;
                                }
                            _ => return Err(Error::Custom { error: "SSHDecode currently only implements Unit or single value enum variants. ".into(), span: None})
                            }
                            Ok(())
                        })?;

                    }
                    if let Some(unk) = unknown_arm.take() {
                        match_arm.append(unk);
                    }
                }
                Ok(())
            })?;
            fn_body.push_parsed("; Ok(r)")?;
            Ok(())
        })?;
    Ok(())
}
