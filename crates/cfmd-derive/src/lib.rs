//! Compile-time schema generation for the public CFMD Rust SDK.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, Expr, ExprLit, ExprUnary, Field, Fields, GenericArgument, Ident, Lit,
    LitStr, PathArguments, Type, UnOp, parse_macro_input,
};

#[proc_macro_derive(CfmdSchema, attributes(cfmd))]
pub fn derive_cfmd_schema(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_schema(&input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn expand_schema(input: &DeriveInput) -> syn::Result<TokenStream2> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "CfmdSchema does not support generic schema roots",
        ));
    }
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "CfmdSchema can only be derived for structs",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &data.fields,
            "CfmdSchema requires a struct with named EntitySet<T> fields",
        ));
    };

    let name = &input.ident;
    let contract_schema_revision = parse_schema_contract_revision(input)?;
    let mut members = Vec::with_capacity(fields.named.len());
    let mut seen = std::collections::BTreeSet::new();
    for field in &fields.named {
        let Some(ident) = field.ident.as_ref() else {
            return Err(syn::Error::new_spanned(
                field,
                "CfmdSchema requires named fields",
            ));
        };
        let Some(target) = generic_target(&field.ty, "EntitySet") else {
            return Err(syn::Error::new_spanned(
                &field.ty,
                "CfmdSchema fields must have type EntitySet<T>",
            ));
        };
        let target_key = quote!(#target).to_string();
        if !seen.insert(target_key) {
            return Err(syn::Error::new_spanned(
                &field.ty,
                "the same entity type cannot appear in a CfmdSchema more than once",
            ));
        }
        members.push((ident, target.clone()));
    }

    let register = members.iter().map(|(_, target)| {
        quote! {
            let builder = builder.object::<#target>();
        }
    });
    let bind = members.iter().map(|(ident, target)| {
        quote! {
            #ident: ::cfmd::EntitySet::<#target>::__bind(::std::sync::Arc::clone(&source))?,
        }
    });
    let authority = schema_authority_tree(
        &members
            .iter()
            .map(|(_, target)| target.clone())
            .collect::<Vec<_>>(),
    );
    let contract_revision = match contract_schema_revision {
        Some(revision) => quote! {
            fn contract_schema_revision() -> ::std::option::Option<u64> {
                ::std::option::Option::Some(#revision)
            }
        },
        None => quote! {},
    };

    Ok(quote! {
        impl ::cfmd::CfmdSchema for #name {
            type DefinitionAuthority = #authority;

            fn definition() -> ::cfmd::Result<::cfmd::Schema> {
                let builder = ::cfmd::Schema::builder();
                #(#register)*
                builder.build()
            }

            #contract_revision

            #[doc(hidden)]
            fn __bind(source: ::std::sync::Arc<::cfmd::__private::ContextSource>) -> ::cfmd::Result<Self> {
                Ok(Self {
                    #(#bind)*
                })
            }
        }
    })
}

fn parse_schema_contract_revision(input: &DeriveInput) -> syn::Result<Option<syn::LitInt>> {
    let mut revision = None;
    for attr in &input.attrs {
        if !attr.path().is_ident("cfmd") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("schema_revision") {
                if revision.is_some() {
                    return Err(meta.error("duplicate schema_revision"));
                }
                revision = Some(meta.value()?.parse::<syn::LitInt>()?);
                return Ok(());
            }
            Err(meta.error("unsupported schema option; expected schema_revision"))
        })?;
    }
    Ok(revision)
}

fn schema_authority_tree(types: &[Type]) -> TokenStream2 {
    match types {
        [] => quote!(::cfmd::__private::SchemaAuthorityEmpty),
        [single] => quote!(::cfmd::__private::SchemaAuthorityLeaf<#single>),
        _ => {
            let midpoint = types.len() / 2;
            let left = schema_authority_tree(&types[..midpoint]);
            let right = schema_authority_tree(&types[midpoint..]);
            quote!(::cfmd::__private::SchemaAuthorityPair<#left, #right>)
        }
    }
}

#[proc_macro_derive(CfmdEntity, attributes(cfmd))]
pub fn derive_cfmd_entity(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_entity(&input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

struct EntityOptions {
    key: LitStr,
    proxy: Ident,
    many: Vec<ManyDeclaration>,
    authoritative: bool,
}

#[derive(Clone)]
struct ManyDeclaration {
    name: Ident,
    target: Type,
    via: Option<Ident>,
    owned: bool,
    orphan_delete: bool,
}

#[derive(Clone)]
enum FieldKind {
    Identity,
    Value,
    Reference(Type),
    OptionalReference(Type),
    VirtualMany {
        target: Type,
        via: Option<Ident>,
        owned: bool,
        orphan_delete: bool,
    },
}

struct EntityField<'a> {
    field: &'a Field,
    ident: &'a Ident,
    kind: FieldKind,
    semantic_name: Option<LitStr>,
    rules: Vec<FieldRuleSpec>,
}

#[derive(Clone)]
enum FieldRuleSpec {
    I64Range {
        min: Option<i64>,
        max: Option<i64>,
    },
    TextLength {
        min: Option<usize>,
        max: Option<usize>,
    },
    TextOneOf(Vec<LitStr>),
    TextMatches(Expr),
}

#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operator or protocol case analysis together."
)]
fn expand_entity(input: &DeriveInput) -> syn::Result<TokenStream2> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "CfmdEntity does not support generic entity structs; use concrete domain types",
        ));
    }

    let name = &input.ident;
    let options = parse_entity_options(input)?;
    let fields = named_fields(input)?;
    let entity_fields = classify_fields(name, fields)?;
    validate_identity_count(name, &entity_fields)?;
    validate_semantic_field_names(&entity_fields)?;
    if options.authoritative
        && let Some(bound) = entity_fields
            .iter()
            .find(|field| field.semantic_name.is_some())
    {
        return Err(syn::Error::new_spanned(
            bound.field,
            "#[cfmd(bind = ...)] is not permitted on an authoritative entity; authoritative entities must match the current persisted schema exactly and schema evolution must use a migration",
        ));
    }
    let stored_fields = entity_fields
        .iter()
        .filter(|field| !matches!(field.kind, FieldKind::VirtualMany { .. }))
        .collect::<Vec<_>>();
    let virtual_fields = entity_fields
        .iter()
        .filter(|field| matches!(field.kind, FieldKind::VirtualMany { .. }))
        .collect::<Vec<_>>();
    let authoritative = options.authoritative;
    let mut many = options.many;
    many.extend(virtual_fields.iter().map(|entry| {
        let FieldKind::VirtualMany {
            target,
            via,
            owned,
            orphan_delete,
        } = &entry.kind
        else {
            unreachable!("virtual field filter keeps only Many<T>");
        };
        ManyDeclaration {
            name: entry.ident.clone(),
            target: target.clone(),
            via: via.clone(),
            owned: *owned,
            orphan_delete: *orphan_delete,
        }
    }));
    validate_many_declarations(&stored_fields, &many)?;

    let proxy = options.proxy;
    let key = options.key;
    let authoritative_impl = authoritative.then(|| {
        quote! {
            impl ::cfmd::__private::AuthoritativeObject for #name {}
        }
    });
    let entity_vis = &input.vis;
    let identity = entity_fields
        .iter()
        .find(|field| matches!(field.kind, FieldKind::Identity))
        .expect("identity count was validated");
    let row_codec = row_codec_tokens(name, &stored_fields, &virtual_fields);
    let constructor = constructor_tokens(name, entity_vis, &stored_fields, &virtual_fields);
    let bind_relations =
        bind_relations_tokens(name, identity.ident, &stored_fields, &virtual_fields);
    let identity_method = identity_method_tokens(identity.ident);
    let append_insert = append_insert_tokens(name, identity.ident, &stored_fields, &virtual_fields);
    let field_schema = stored_fields.iter().map(|entry| {
        let ident = entry.ident;
        let ty = &entry.field.ty;
        let semantic_name = entry
            .semantic_name
            .as_ref()
            .map(|name| quote!(.__with_semantic_name(#name)));
        let rules = entry.rules.iter().map(field_rule_tokens);
        quote!(
            ::cfmd::ObjectFieldSchema::of::<#ty>(stringify!(#ident))
                #semantic_name
                .__with_rules(vec![#(#rules),*])
        )
    });
    let accessors = stored_fields
        .iter()
        .map(|entry| accessor_tokens(name, entry));
    let path_ident = format_ident!("{}Path", name);
    let path_accessors = stored_fields
        .iter()
        .filter_map(|entry| path_accessor_tokens(name, entry));
    let many_accessors = many
        .iter()
        .map(|entry| many_accessor_tokens(name, entity_vis, entry));
    let many_schema = many.iter().map(|entry| many_schema_tokens(name, entry));

    Ok(quote! {
        #row_codec

        #[derive(Debug, Clone)]
        #entity_vis struct #path_ident<S: ::cfmd::Object> {
            inner: ::cfmd::RefPath<S, #name>,
        }

        impl<S: ::cfmd::Object> ::cfmd::ObjectPatchField<S, ::cfmd::Ref<#name>> for #path_ident<S> {
            fn into_patch_field(self) -> ::cfmd::Result<::cfmd::Field<S, ::cfmd::Ref<#name>>> {
                ::cfmd::ObjectPatchField::into_patch_field(self.inner)
            }
        }

        impl<S: ::cfmd::Object> #path_ident<S> {
            #[doc(hidden)]
            #[must_use]
            pub fn __from_inner(inner: ::cfmd::RefPath<S, #name>) -> Self {
                Self { inner }
            }

            #[must_use]
            pub fn eq(self, target: ::cfmd::Id<#name>)
                -> ::cfmd::PathPredicate<S, #name, ::cfmd::EqPredicate<#name>>
            {
                self.inner.eq(target)
            }

            #[must_use]
            pub fn matches<F, P>(self, predicate: F) -> ::cfmd::PathPredicate<S, #name, P>
            where
                F: FnOnce(&<#name as ::cfmd::Object>::Proxy) -> P,
                P: ::cfmd::ObjectPredicate<#name>,
            {
                self.inner.matches(predicate)
            }

            #(#path_accessors)*
        }

        #[derive(Debug, Clone)]
        #entity_vis struct #proxy {
            inner: ::cfmd::ObjectProxy<#name>,
        }

        impl #proxy {
            #(#accessors)*
            #(#many_accessors)*
        }

        impl ::cfmd::Object for #name {
            type Proxy = #proxy;
            const KEY: &'static str = #key;

            fn fields() -> Vec<::cfmd::ObjectFieldSchema> {
                vec![#(#field_schema),*]
            }

            fn many_fields() -> Vec<::cfmd::ObjectManyFieldSchema> {
                vec![#(#many_schema),*]
            }

            #bind_relations
            #identity_method
            #append_insert

            fn proxy(relation: ::cfmd::__private::Relation<Self>) -> Self::Proxy {
                #proxy {
                    inner: ::cfmd::ObjectProxy::__new(relation),
                }
            }
        }

        #authoritative_impl

        #constructor
    })
}

fn row_codec_tokens(
    name: &Ident,
    stored_fields: &[&EntityField<'_>],
    virtual_fields: &[&EntityField<'_>],
) -> TokenStream2 {
    let into_values = stored_fields.iter().map(|entry| {
        let ident = entry.ident;
        quote!(::cfmd::__private::ValueCodec::into_value(self.#ident))
    });
    let from_values = stored_fields.iter().map(|entry| {
        let ident = entry.ident;
        let ty = &entry.field.ty;
        quote! {
            #ident: <#ty as ::cfmd::__private::ValueCodec>::from_value(
                values.next().ok_or_else(|| ::cfmd::__private::row_shape_error(stringify!(#name)))?
            )?
        }
    });
    let from_virtual = virtual_fields.iter().map(|entry| {
        let ident = entry.ident;
        match &entry.kind {
            FieldKind::VirtualMany { owned: true, .. } => {
                quote!(#ident: ::cfmd::OwnedMany::__new())
            }
            FieldKind::VirtualMany { owned: false, .. } => quote!(#ident: ::cfmd::Many::__new()),
            _ => unreachable!("virtual field list contains only relationship collections"),
        }
    });
    let accepts = stored_fields.iter().map(|entry| {
        let ty = &entry.field.ty;
        quote! {
            match types.next() {
                Some(ty) if <#ty as ::cfmd::__private::ValueCodec>::accepts(ty) => {}
                _ => return false,
            }
        }
    });
    quote! {
        impl ::cfmd::__private::RowCodec for #name {
            fn into_row(self) -> ::cfmd::__private::Row {
                vec![#(#into_values),*]
            }

            fn from_row(row: &::cfmd::__private::Row) -> ::cfmd::Result<Self> {
                let mut values = row.iter();
                let value = Self {
                    #(#from_values),*,
                    #(#from_virtual),*
                };
                if values.next().is_some() {
                    return Err(::cfmd::__private::row_shape_error(stringify!(#name)));
                }
                Ok(value)
            }

            fn accepts(types: &[::cfmd::__private::Type]) -> bool {
                let mut types = types.iter();
                #(#accepts)*
                types.next().is_none()
            }
        }
    }
}

fn bind_relations_tokens(
    source: &Ident,
    identity: &Ident,
    stored_fields: &[&EntityField<'_>],
    virtual_fields: &[&EntityField<'_>],
) -> TokenStream2 {
    let references = stored_fields.iter().filter_map(|entry| match &entry.kind {
        FieldKind::Reference(_) => {
            let ident = entry.ident;
            Some(quote!(self.#ident.__bind(context.clone());))
        }
        FieldKind::OptionalReference(_) => {
            let ident = entry.ident;
            Some(quote! {
                if let Some(reference) = self.#ident.as_mut() {
                    reference.__bind(context.clone());
                }
            })
        }
        _ => None,
    });
    let references = references.collect::<Vec<_>>();
    if virtual_fields.is_empty() && references.is_empty() {
        return quote! {};
    }
    let bindings = virtual_fields.iter().map(|entry| {
        let ident = entry.ident;
        let FieldKind::VirtualMany {
            target,
            owned,
            orphan_delete,
            ..
        } = &entry.kind
        else {
            unreachable!("virtual field list contains only relationship collections");
        };
        if *owned {
            let policy = if *orphan_delete {
                quote!(::cfmd::OrphanPolicy::DeleteIfUnowned)
            } else {
                quote!(::cfmd::OrphanPolicy::Keep)
            };
            quote! {
                self.#ident.__bind(
                    context.clone(),
                    ::cfmd::__private::many_relation_id::<#source>(stringify!(#ident)),
                    source_id,
                    <#source as ::cfmd::Object>::type_id(),
                    ::cfmd::__private::identity_equivalence_id::<#source>()?,
                    ::cfmd::__private::identity_equivalence_id::<#target>()?,
                );
                self.#ident.__set_orphan_policy(#policy);
            }
        } else {
            quote! {
                self.#ident.__bind(
                    context.clone(),
                    ::cfmd::__private::many_relation_id::<#source>(stringify!(#ident)),
                    source_id,
                    <#source as ::cfmd::Object>::type_id(),
                    ::cfmd::__private::identity_equivalence_id::<#source>()?,
                    ::cfmd::__private::identity_equivalence_id::<#target>()?,
                );
            }
        }
    });
    quote! {
        fn __bind_relations(&mut self, context: &::cfmd::__private::ReadContext) -> ::cfmd::Result<()> {
            let source_id = self.#identity.raw();
            #(#references)*
            #(#bindings)*
            Ok(())
        }
    }
}

fn identity_method_tokens(identity: &Ident) -> TokenStream2 {
    quote! {
        fn __identity_raw(&self) -> ::cfmd::Result<u128> {
            Ok(self.#identity.raw())
        }
    }
}

fn append_insert_tokens(
    source: &Ident,
    identity: &Ident,
    _stored_fields: &[&EntityField<'_>],
    virtual_fields: &[&EntityField<'_>],
) -> TokenStream2 {
    if virtual_fields.is_empty() {
        return quote! {};
    }
    let insert_edges = virtual_fields
        .iter()
        .map(|entry| {
            let ident = entry.ident;
            let FieldKind::VirtualMany { target, owned, orphan_delete, .. } = &entry.kind else {
                unreachable!("virtual field list contains only relationship collections");
            };
            let register = if *owned {
                let policy = if *orphan_delete { quote!(::cfmd::OrphanPolicy::DeleteIfUnowned) } else { quote!(::cfmd::OrphanPolicy::Keep) };
                quote!(::cfmd::__private::register_owned_many::<#source, #target>(plan, stringify!(#ident), #policy)?;)
            } else { TokenStream2::new() };
            quote! {
                #register
                for target in ::core::mem::take(&mut self.#ident).__into_pending()? {
                    let target_id = <#target as ::cfmd::Object>::__identity_raw(&target)?;
                    <#target as ::cfmd::Object>::__append_insert(target, context, plan)?;
                    ::cfmd::__private::append_many_edge::<#source, #target>(
                        plan,
                        stringify!(#ident),
                        source_id,
                        target_id,
                    )?;
                }
            }
        })
        .collect::<Vec<_>>();
    let update_edges = virtual_fields.iter().map(|entry| {
        let ident = entry.ident;
        let FieldKind::VirtualMany { target, owned, orphan_delete, .. } = &entry.kind else {
            unreachable!("virtual field list contains only relationship collections");
        };
        let register = if *owned {
            let policy = if *orphan_delete { quote!(::cfmd::OrphanPolicy::DeleteIfUnowned) } else { quote!(::cfmd::OrphanPolicy::Keep) };
            quote!(::cfmd::__private::register_owned_many::<#source, #target>(plan, stringify!(#ident), #policy)?;)
        } else { TokenStream2::new() };
        quote! {
            #register
            let relation = ::cfmd::__private::many_relation_id::<#source>(stringify!(#ident));
            if !self.#ident.__preserves_binding(context, relation, source_id)? {
                ::cfmd::__private::append_remove_many_edges::<#source>(
                    context,
                    plan,
                    stringify!(#ident),
                    source_id,
                )?;
                for target in ::core::mem::take(&mut self.#ident).__into_pending()? {
                    let target_id = <#target as ::cfmd::Object>::__identity_raw(&target)?;
                    <#target as ::cfmd::Object>::__append_insert(target, context, plan)?;
                    ::cfmd::__private::append_many_edge::<#source, #target>(
                        plan,
                        stringify!(#ident),
                        source_id,
                        target_id,
                    )?;
                }
            }
        }
    });
    quote! {
        fn __append_relationships(
            &mut self,
            context: &::cfmd::__private::ReadContext,
            plan: &mut ::cfmd::__private::Plan,
        ) -> ::cfmd::Result<()> {
            let source_id = self.#identity.raw();
            #(#insert_edges)*
            Ok(())
        }

        fn __append_update_relationships(
            &mut self,
            context: &::cfmd::__private::ReadContext,
            plan: &mut ::cfmd::__private::Plan,
        ) -> ::cfmd::Result<()> {
            let source_id = self.#identity.raw();
            #(#update_edges)*
            Ok(())
        }
    }
}

fn constructor_tokens(
    name: &Ident,
    visibility: &syn::Visibility,
    stored_fields: &[&EntityField<'_>],
    virtual_fields: &[&EntityField<'_>],
) -> Option<TokenStream2> {
    if virtual_fields.is_empty() {
        return None;
    }
    let fields = stored_fields
        .iter()
        .copied()
        .chain(virtual_fields.iter().copied())
        .collect::<Vec<_>>();
    let parameters = fields.iter().map(|entry| {
        let ident = entry.ident;
        let ty = &entry.field.ty;
        quote!(#ident: #ty)
    });
    let initializers = fields.iter().map(|entry| entry.ident);
    Some(quote! {
        impl #name {
            /// Constructs a detached object graph. Relationship values are lowered into a `Plan`
            /// only when this object is inserted through CFMD.
            #[must_use]
            #visibility fn cfmd_new(#(#parameters),*) -> Self {
                Self { #(#initializers),* }
            }
        }
    })
}

fn many_accessor_tokens(
    source: &Ident,
    visibility: &syn::Visibility,
    declaration: &ManyDeclaration,
) -> TokenStream2 {
    let accessor = &declaration.name;
    let target = &declaration.target;
    quote! {
        #[must_use]
        #visibility fn #accessor(&self) -> ::cfmd::ManyField<#source, #target> {
            self.inner.__many::<#target>(stringify!(#accessor))
        }
    }
}

fn many_schema_tokens(source: &Ident, declaration: &ManyDeclaration) -> TokenStream2 {
    let accessor = &declaration.name;
    let target = &declaration.target;
    if declaration.owned {
        let policy = if declaration.orphan_delete {
            quote!(::cfmd::OrphanPolicy::DeleteIfUnowned)
        } else {
            quote!(::cfmd::OrphanPolicy::Keep)
        };
        return quote!(::cfmd::ObjectManyFieldSchema::owned::<#source, #target>(stringify!(#accessor), #policy));
    }
    if let Some(via) = &declaration.via {
        quote! {
            ::cfmd::ObjectManyFieldSchema::of::<#source, #target>(
                stringify!(#accessor),
                stringify!(#via),
            )
        }
    } else {
        quote! {
            ::cfmd::ObjectManyFieldSchema::inferred::<#source, #target>(stringify!(#accessor))
        }
    }
}

fn validate_identity_count(name: &Ident, fields: &[EntityField<'_>]) -> syn::Result<()> {
    let identity_count = fields
        .iter()
        .filter(|field| matches!(field.kind, FieldKind::Identity))
        .count();
    if identity_count == 1 {
        return Ok(());
    }
    Err(syn::Error::new_spanned(
        name,
        format!("CfmdEntity requires exactly one #[cfmd(id)] field; found {identity_count}"),
    ))
}

fn accessor_tokens(name: &Ident, entry: &EntityField<'_>) -> TokenStream2 {
    let ident = entry.ident;
    let vis = &entry.field.vis;
    match &entry.kind {
        FieldKind::Identity | FieldKind::Value => {
            let ty = &entry.field.ty;
            quote! {
                #[must_use]
                #vis fn #ident(&self) -> ::cfmd::Field<#name, #ty> {
                    self.inner.__field::<#ty>(stringify!(#ident))
                }
            }
        }
        FieldKind::Reference(target) => {
            let path = entity_path_type(target);
            let rule_ident = format_ident!("{}_rule", ident);
            quote! {
                #[must_use]
                #vis fn #ident(&self) -> #path<#name> {
                    #path::__from_inner(
                        self.inner.__ref::<#target>(stringify!(#ident)).__path()
                    )
                }

                #[must_use]
                #vis fn #rule_ident(&self) -> ::cfmd::ObjectRuleField<#name, ::cfmd::Ref<#target>> {
                    self.inner.__ref_rule::<#target>(stringify!(#ident))
                }
            }
        }
        FieldKind::OptionalReference(target) => {
            let rule_ident = format_ident!("{}_rule", ident);
            quote! {
                #[must_use]
                #vis fn #ident(&self) -> ::cfmd::OptionalRefField<#name, #target> {
                    self.inner.__optional_ref::<#target>(stringify!(#ident))
                }

                #[must_use]
                #vis fn #rule_ident(&self) -> ::cfmd::ObjectRuleField<#name, Option<::cfmd::Ref<#target>>> {
                    self.inner.__optional_ref_rule::<#target>(stringify!(#ident))
                }
            }
        }
        FieldKind::VirtualMany { .. } => TokenStream2::new(),
    }
}

fn entity_path_type(target: &Type) -> TokenStream2 {
    let Type::Path(value) = target else {
        return quote!(::cfmd::RefPath);
    };
    let mut path = value.path.clone();
    let last = path
        .segments
        .last_mut()
        .expect("reference target path must have one segment");
    last.ident = format_ident!("{}Path", last.ident);
    last.arguments = syn::PathArguments::None;
    quote!(#path)
}

fn path_accessor_tokens(source: &Ident, entry: &EntityField<'_>) -> Option<TokenStream2> {
    let ident = entry.ident;
    let vis = &entry.field.vis;
    match &entry.kind {
        FieldKind::Identity | FieldKind::Value => {
            let ty = &entry.field.ty;
            Some(quote! {
                #[must_use]
                #vis fn #ident(&self) -> ::cfmd::PathField<S, #source, #ty> {
                    self.inner.__field::<#ty>(stringify!(#ident))
                }
            })
        }
        FieldKind::Reference(target) => {
            let target_path = entity_path_type(target);
            Some(quote! {
                #[must_use]
                #vis fn #ident(&self) -> #target_path<S> {
                    #target_path::__from_inner(
                        self.inner.__ref::<#target>(stringify!(#ident))
                    )
                }
            })
        }
        // Optional references need an explicit nullable-path law; do not silently turn None into
        // an inner join in the first traversal pass.
        FieldKind::OptionalReference(_) | FieldKind::VirtualMany { .. } => None,
    }
}

fn parse_entity_options(input: &DeriveInput) -> syn::Result<EntityOptions> {
    let mut key = None;
    let mut proxy = None;
    let mut many = Vec::new();
    let mut authoritative = false;
    for attr in &input.attrs {
        if !attr.path().is_ident("cfmd") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("key") {
                if key.is_some() {
                    return Err(meta.error("duplicate cfmd key"));
                }
                key = Some(meta.value()?.parse::<LitStr>()?);
                return Ok(());
            }
            if meta.path.is_ident("proxy") {
                if proxy.is_some() {
                    return Err(meta.error("duplicate cfmd proxy"));
                }
                let value = meta.value()?.parse::<LitStr>()?;
                proxy = Some(Ident::new(&value.value(), value.span()));
                return Ok(());
            }
            if meta.path.is_ident("many") {
                many.push(parse_many_declaration(&meta)?);
                return Ok(());
            }
            if meta.path.is_ident("authoritative") {
                if authoritative {
                    return Err(meta.error("duplicate authoritative marker"));
                }
                authoritative = true;
                return Ok(());
            }
            Err(meta.error(
                "unsupported entity option; expected key, proxy, authoritative, or many(...)",
            ))
        })?;
    }
    let key = key.ok_or_else(|| {
        syn::Error::new_spanned(
            &input.ident,
            "CfmdEntity requires #[cfmd(key = \"application.stable-key\")]",
        )
    })?;
    let proxy = proxy.unwrap_or_else(|| format_ident!("{}Fields", input.ident));
    Ok(EntityOptions {
        key,
        proxy,
        many,
        authoritative,
    })
}

fn parse_many_declaration(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<ManyDeclaration> {
    let mut name = None;
    let mut target = None;
    let mut via = None;
    meta.parse_nested_meta(|item| {
        if item.path.is_ident("name") {
            if name.is_some() {
                return Err(item.error("duplicate many relationship name"));
            }
            name = Some(item.value()?.parse::<Ident>()?);
            return Ok(());
        }
        if item.path.is_ident("target") {
            if target.is_some() {
                return Err(item.error("duplicate many relationship target"));
            }
            target = Some(item.value()?.parse::<Type>()?);
            return Ok(());
        }
        if item.path.is_ident("via") {
            if via.is_some() {
                return Err(item.error("duplicate many relationship via field"));
            }
            via = Some(item.value()?.parse::<Ident>()?);
            return Ok(());
        }
        Err(item.error("unsupported many option; expected name, target, or via"))
    })?;
    let name = name.ok_or_else(|| meta.error("many(...) requires name = <identifier>"))?;
    let target = target.ok_or_else(|| meta.error("many(...) requires target = <Type>"))?;
    let via = via.ok_or_else(|| meta.error("many(...) requires via = <reference-field>"))?;
    Ok(ManyDeclaration {
        name,
        target,
        via: Some(via),
        owned: false,
        orphan_delete: false,
    })
}

fn validate_many_declarations(
    fields: &[&EntityField<'_>],
    many: &[ManyDeclaration],
) -> syn::Result<()> {
    let mut names = std::collections::BTreeSet::new();
    for declaration in many {
        let name = declaration.name.to_string();
        if fields.iter().any(|field| field.ident == &declaration.name) {
            return Err(syn::Error::new_spanned(
                &declaration.name,
                format!("reverse-many accessor `{name}` conflicts with stored field `{name}`"),
            ));
        }
        if !names.insert(name.clone()) {
            return Err(syn::Error::new_spanned(
                &declaration.name,
                format!("duplicate reverse-many accessor `{name}`"),
            ));
        }
    }
    Ok(())
}

fn named_fields(
    input: &DeriveInput,
) -> syn::Result<&syn::punctuated::Punctuated<Field, syn::Token![,]>> {
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "CfmdEntity can only be derived for structs",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &data.fields,
            "CfmdEntity requires a struct with named fields",
        ));
    };
    Ok(&fields.named)
}

fn classify_fields<'a>(
    entity: &Ident,
    fields: &'a syn::punctuated::Punctuated<Field, syn::Token![,]>,
) -> syn::Result<Vec<EntityField<'a>>> {
    let mut output = Vec::with_capacity(fields.len());
    for field in fields {
        let Some(ident) = field.ident.as_ref() else {
            return Err(syn::Error::new_spanned(
                field,
                "CfmdEntity requires named fields",
            ));
        };
        let options = parse_field_options(field)?;
        let marked_id = options.id;
        let kind = if marked_id {
            if !is_id_of(&field.ty, entity) {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    format!(
                        "#[cfmd(id)] field must have type Id<{entity}> so identity is strongly typed"
                    ),
                ));
            }
            FieldKind::Identity
        } else if is_outer_named_type(&field.ty, "Id") {
            return Err(syn::Error::new_spanned(
                field,
                "Id<T> fields must explicitly declare #[cfmd(id)]",
            ));
        } else if let Some(target) = generic_target(&field.ty, "OwnedMany") {
            if options.via.is_some() {
                return Err(syn::Error::new_spanned(
                    field,
                    "#[cfmd(via = ...)] is not valid on OwnedMany<T>",
                ));
            }
            FieldKind::VirtualMany {
                target: target.clone(),
                via: None,
                owned: true,
                orphan_delete: options.orphan_delete,
            }
        } else if let Some(target) = generic_target(&field.ty, "Many") {
            if options.orphan_delete {
                return Err(syn::Error::new_spanned(
                    field,
                    "#[cfmd(orphan = \"delete\")] requires OwnedMany<T>",
                ));
            }
            FieldKind::VirtualMany {
                target: target.clone(),
                via: options.via,
                owned: false,
                orphan_delete: false,
            }
        } else if options.via.is_some() || options.orphan_delete {
            return Err(syn::Error::new_spanned(
                field,
                "#[cfmd(via = ...)] is only valid on Many<T> virtual relationship fields",
            ));
        } else if let Some(target) = generic_target(&field.ty, "Ref") {
            FieldKind::Reference(target.clone())
        } else if let Some(target) = optional_ref_target(&field.ty) {
            FieldKind::OptionalReference(target.clone())
        } else {
            FieldKind::Value
        };
        if matches!(kind, FieldKind::VirtualMany { .. }) && options.bind.is_some() {
            return Err(syn::Error::new_spanned(
                field,
                "#[cfmd(bind = ...)] is only valid on stored fields",
            ));
        }
        if let Some(bind) = &options.bind {
            if bind.value().is_empty() {
                return Err(syn::Error::new_spanned(bind, "bind cannot be empty"));
            }
            if *ident == bind.value() {
                return Err(syn::Error::new_spanned(
                    bind,
                    "bind must name a different persisted field name",
                ));
            }
        }
        validate_field_rules(field, &kind, &options.rules)?;
        output.push(EntityField {
            field,
            ident,
            kind,
            semantic_name: options.bind,
            rules: options.rules,
        });
    }
    Ok(output)
}

struct FieldOptions {
    id: bool,
    via: Option<Ident>,
    orphan_delete: bool,
    bind: Option<LitStr>,
    rules: Vec<FieldRuleSpec>,
}

#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operator or protocol case analysis together."
)]
fn parse_field_options(field: &Field) -> syn::Result<FieldOptions> {
    let mut id = false;
    let mut via = None;
    let mut orphan_delete = false;
    let mut bind = None;
    let mut rules = Vec::new();
    for attr in &field.attrs {
        if !attr.path().is_ident("cfmd") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("id") {
                if id {
                    return Err(meta.error("duplicate cfmd id marker"));
                }
                id = true;
                return Ok(());
            }
            if meta.path.is_ident("via") {
                if via.is_some() {
                    return Err(meta.error("duplicate cfmd via marker"));
                }
                via = Some(meta.value()?.parse::<Ident>()?);
                return Ok(());
            }
            if meta.path.is_ident("orphan") {
                let value = meta.value()?.parse::<LitStr>()?;
                if value.value() != "delete" {
                    return Err(meta.error("orphan policy must be \"delete\""));
                }
                orphan_delete = true;
                return Ok(());
            }
            if meta.path.is_ident("bind") {
                if bind.is_some() {
                    return Err(meta.error("duplicate bind marker"));
                }
                bind = Some(meta.value()?.parse::<LitStr>()?);
                return Ok(());
            }
            if meta.path.is_ident("range") {
                let mut min = None;
                let mut max = None;
                meta.parse_nested_meta(|item| {
                    if item.path.is_ident("min") {
                        if min.is_some() {
                            return Err(item.error("duplicate range min"));
                        }
                        min = Some(parse_i64_literal(item.value()?.parse::<Expr>()?)?);
                        return Ok(());
                    }
                    if item.path.is_ident("max") {
                        if max.is_some() {
                            return Err(item.error("duplicate range max"));
                        }
                        max = Some(parse_i64_literal(item.value()?.parse::<Expr>()?)?);
                        return Ok(());
                    }
                    Err(item.error("unsupported range option; expected min or max"))
                })?;
                if min.is_none() && max.is_none() {
                    return Err(meta.error("range(...) requires min and/or max"));
                }
                rules.push(FieldRuleSpec::I64Range { min, max });
                return Ok(());
            }
            if meta.path.is_ident("length") {
                let mut min = None;
                let mut max = None;
                meta.parse_nested_meta(|item| {
                    if item.path.is_ident("min") {
                        if min.is_some() {
                            return Err(item.error("duplicate length min"));
                        }
                        min = Some(parse_usize_literal(item.value()?.parse::<Expr>()?)?);
                        return Ok(());
                    }
                    if item.path.is_ident("max") {
                        if max.is_some() {
                            return Err(item.error("duplicate length max"));
                        }
                        max = Some(parse_usize_literal(item.value()?.parse::<Expr>()?)?);
                        return Ok(());
                    }
                    Err(item.error("unsupported length option; expected min or max"))
                })?;
                if min.is_none() && max.is_none() {
                    return Err(meta.error("length(...) requires min and/or max"));
                }
                rules.push(FieldRuleSpec::TextLength { min, max });
                return Ok(());
            }
            if meta.path.is_ident("one_of") {
                let content;
                syn::parenthesized!(content in meta.input);
                let values = content
                    .parse_terminated(syn::parse::ParseBuffer::parse::<LitStr>, syn::Token![,])?
                    .into_iter()
                    .collect::<Vec<_>>();
                if values.is_empty() {
                    return Err(meta.error("one_of(...) requires at least one string literal"));
                }
                rules.push(FieldRuleSpec::TextOneOf(values));
                return Ok(());
            }
            if meta.path.is_ident("matches") {
                let pattern = meta.value()?.parse::<Expr>()?;
                rules.push(FieldRuleSpec::TextMatches(pattern));
                return Ok(());
            }
            Err(meta.error(
                "unsupported field option; expected id, via, orphan, bind, range(...), length(...), one_of(...), or matches = <TextPattern expression>"
            ))
        })?;
    }
    Ok(FieldOptions {
        id,
        via,
        orphan_delete,
        bind,
        rules,
    })
}

fn validate_semantic_field_names(fields: &[EntityField<'_>]) -> syn::Result<()> {
    let mut names = std::collections::BTreeMap::<String, &Ident>::new();
    for field in fields {
        if matches!(field.kind, FieldKind::VirtualMany { .. }) {
            continue;
        }
        let semantic_name = field
            .semantic_name
            .as_ref()
            .map_or_else(|| field.ident.to_string(), LitStr::value);
        if let Some(previous) = names.insert(semantic_name.clone(), field.ident) {
            return Err(syn::Error::new_spanned(
                field.field,
                format!(
                    "semantic field name `{semantic_name}` is already used by `{previous}`; bound fields must resolve to unique persisted identities"
                ),
            ));
        }
    }
    Ok(())
}

fn validate_field_rules(
    field: &Field,
    kind: &FieldKind,
    rules: &[FieldRuleSpec],
) -> syn::Result<()> {
    if rules.is_empty() {
        return Ok(());
    }
    if !matches!(kind, FieldKind::Value) {
        return Err(syn::Error::new_spanned(
            field,
            "CFMD value rules can only be attached to stored scalar value fields",
        ));
    }
    for rule in rules {
        match rule {
            FieldRuleSpec::I64Range { .. } if !is_exact_type(&field.ty, "i64") => {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "#[cfmd(range(...))] requires an i64 field",
                ));
            }
            FieldRuleSpec::TextLength { .. }
            | FieldRuleSpec::TextOneOf(_)
            | FieldRuleSpec::TextMatches(_)
                if !is_exact_type(&field.ty, "String") =>
            {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "#[cfmd(length(...))], #[cfmd(one_of(...))], and #[cfmd(matches = ...)] require a String field",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn field_rule_tokens(rule: &FieldRuleSpec) -> TokenStream2 {
    match rule {
        FieldRuleSpec::I64Range { min, max } => {
            let min = min
                .as_ref()
                .map_or_else(|| quote!(None), |value| quote!(Some(#value)));
            let max = max
                .as_ref()
                .map_or_else(|| quote!(None), |value| quote!(Some(#value)));
            quote!(::cfmd::FieldRule::I64Range { min: #min, max: #max })
        }
        FieldRuleSpec::TextLength { min, max } => {
            let min = min
                .as_ref()
                .map_or_else(|| quote!(0usize), |value| quote!(#value));
            let max = max
                .as_ref()
                .map_or_else(|| quote!(None), |value| quote!(Some(#value)));
            quote!(::cfmd::FieldRule::TextLength { min: #min, max: #max })
        }
        FieldRuleSpec::TextOneOf(values) => quote!(
            ::cfmd::FieldRule::TextOneOf(
                [#(::std::string::String::from(#values)),*].into_iter().collect()
            )
        ),
        FieldRuleSpec::TextMatches(pattern) => {
            quote!(::cfmd::FieldRule::TextMatches(#pattern))
        }
    }
}

fn parse_i64_literal(expr: Expr) -> syn::Result<i64> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Int(value),
            ..
        }) => value.base10_parse::<i64>(),
        Expr::Unary(ExprUnary {
            op: UnOp::Neg(_),
            expr,
            ..
        }) => {
            let Expr::Lit(ExprLit {
                lit: Lit::Int(value),
                ..
            }) = *expr
            else {
                return Err(syn::Error::new_spanned(expr, "expected an integer literal"));
            };
            let magnitude = value.base10_parse::<i64>()?;
            magnitude.checked_neg().ok_or_else(|| {
                syn::Error::new_spanned(value, "integer literal is outside the i64 range")
            })
        }
        other => Err(syn::Error::new_spanned(
            other,
            "CFMD rule bounds must be integer literals, not host-language expressions",
        )),
    }
}

fn parse_usize_literal(expr: Expr) -> syn::Result<usize> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Int(value),
            ..
        }) => value.base10_parse::<usize>(),
        other => Err(syn::Error::new_spanned(
            other,
            "CFMD length bounds must be non-negative integer literals",
        )),
    }
}

fn is_exact_type(ty: &Type, name: &str) -> bool {
    matches!(
        ty,
        Type::Path(path)
            if path.qself.is_none()
                && path.path.segments.len() == 1
                && path.path.segments[0].ident == name
                && matches!(path.path.segments[0].arguments, PathArguments::None)
    )
}

fn is_id_of(ty: &Type, entity: &Ident) -> bool {
    generic_target(ty, "Id").is_some_and(|target| {
        matches!(
            target,
            Type::Path(path)
                if path.qself.is_none()
                    && path.path.segments.len() == 1
                    && path.path.segments[0].ident == *entity
        )
    })
}

fn optional_ref_target(ty: &Type) -> Option<&Type> {
    let inner = generic_target(ty, "Option")?;
    generic_target(inner, "Ref")
}

fn is_outer_named_type(ty: &Type, name: &str) -> bool {
    matches!(
        ty,
        Type::Path(path)
            if path.qself.is_none()
                && path.path.segments.last().is_some_and(|segment| segment.ident == name)
    )
}

fn generic_target<'a>(ty: &'a Type, name: &str) -> Option<&'a Type> {
    let Type::Path(path) = ty else { return None };
    let segment = path.path.segments.last()?;
    if segment.ident != name {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    if arguments.args.len() != 1 {
        return None;
    }
    match arguments.args.first()? {
        GenericArgument::Type(ty) => Some(ty),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::expand_entity;
    use syn::DeriveInput;

    #[test]
    fn missing_key_has_targeted_diagnostic() {
        let input: DeriveInput = syn::parse_quote! {
            struct Todo { #[cfmd(id)] id: Id<Todo> }
        };
        let error = expand_entity(&input).expect_err("missing key must fail");
        assert!(error.to_string().contains("requires #[cfmd(key"));
    }

    #[test]
    fn duplicate_identity_has_targeted_diagnostic() {
        let input: DeriveInput = syn::parse_quote! {
            #[cfmd(key = "todo")]
            struct Todo {
                #[cfmd(id)] id: Id<Todo>,
                #[cfmd(id)] other: Id<Todo>,
            }
        };
        let error = expand_entity(&input).expect_err("two ids must fail");
        assert!(error.to_string().contains("exactly one #[cfmd(id)]"));
    }

    #[test]
    fn reference_shapes_expand() {
        let input: DeriveInput = syn::parse_quote! {
            #[cfmd(key = "task")]
            struct Task {
                #[cfmd(id)] id: Id<Task>,
                owner: Ref<User>,
                reviewer: Option<Ref<User>>,
                title: String,
            }
        };
        let output = expand_entity(&input).expect("valid derive").to_string();
        assert!(output.contains("__ref"));
        assert!(output.contains("__optional_ref"));
        assert!(output.contains("TaskFields"));
    }

    #[test]
    fn reverse_many_expands_without_materialized_field() {
        let input: DeriveInput = syn::parse_quote! {
            #[cfmd(key = "parent")]
            #[cfmd(many(name = children, target = Child, via = parent))]
            struct Parent {
                #[cfmd(id)] id: Id<Parent>,
                name: String,
            }
        };
        let output = expand_entity(&input)
            .expect("valid reverse-many derive")
            .to_string();
        assert!(output.contains("ManyField"));
        assert!(output.contains("ObjectManyFieldSchema"));
        assert!(output.contains("children"));
        assert!(!output.contains("self . children"));
    }

    #[test]
    fn reverse_many_cannot_shadow_stored_field() {
        let input: DeriveInput = syn::parse_quote! {
            #[cfmd(key = "parent")]
            #[cfmd(many(name = children, target = Child, via = parent))]
            struct Parent {
                #[cfmd(id)] id: Id<Parent>,
                children: i64,
            }
        };
        let error = expand_entity(&input).expect_err("shadowed accessor must fail");
        assert!(error.to_string().contains("conflicts with stored field"));
    }

    #[test]
    fn semantic_field_rules_expand_into_object_metadata() {
        let input: DeriveInput = syn::parse_quote! {
            #[cfmd(key = "user")]
            struct User {
                #[cfmd(id)] id: Id<User>,
                #[cfmd(
                    length(min = 3, max = 64),
                    one_of("Artem", "Alice"),
                    matches = ::cfmd::TextPattern::literal("Artem")
                )]
                name: String,
                #[cfmd(range(min = 0, max = 150))]
                age: i64,
            }
        };
        let output = expand_entity(&input)
            .expect("valid rule metadata")
            .to_string();
        assert!(output.contains("TextLength"));
        assert!(output.contains("TextOneOf"));
        assert!(output.contains("I64Range"));
        assert!(output.contains("TextMatches"));
        assert!(output.contains("__with_rules"));
    }

    #[test]
    fn partial_field_bind_targets_explicit_persisted_semantic_name() {
        let input: DeriveInput = syn::parse_quote! {
            #[cfmd(key = "user")]
            struct User {
                #[cfmd(id)] id: Id<User>,
                #[cfmd(bind = "clinical_note")]
                doctor_note: String,
            }
        };
        let output = expand_entity(&input)
            .expect("valid bind metadata")
            .to_string();
        assert!(output.contains("__with_semantic_name"));
        assert!(output.contains("doctor_note"));
        assert!(output.contains("clinical_note"));
    }

    #[test]
    fn bound_fields_cannot_collide_on_persisted_semantic_name() {
        let input: DeriveInput = syn::parse_quote! {
            #[cfmd(key = "user")]
            struct User {
                #[cfmd(id)] id: Id<User>,
                clinical_note: String,
                #[cfmd(bind = "clinical_note")]
                doctor_note: String,
            }
        };
        let error = expand_entity(&input).expect_err("persisted semantic collision must fail");
        assert!(
            error
                .to_string()
                .contains("semantic field name `clinical_note` is already used")
        );
    }

    #[test]
    fn authoritative_entity_rejects_client_side_bind_metadata() {
        let input: DeriveInput = syn::parse_quote! {
            #[cfmd(key = "user", authoritative)]
            struct User {
                #[cfmd(id)] id: Id<User>,
                #[cfmd(bind = "legacy_name")]
                name: String,
            }
        };
        let error = expand_entity(&input).expect_err("authoritative bind must fail");
        assert!(
            error
                .to_string()
                .contains("not permitted on an authoritative entity")
        );
    }

    #[test]
    fn semantic_field_rules_reject_wrong_rust_types() {
        let input: DeriveInput = syn::parse_quote! {
            #[cfmd(key = "user")]
            struct User {
                #[cfmd(id)] id: Id<User>,
                #[cfmd(range(min = 0))]
                name: String,
            }
        };
        let error = expand_entity(&input).expect_err("range on String must fail");
        assert!(error.to_string().contains("requires an i64 field"));
    }

    #[test]
    fn semantic_text_pattern_rule_rejects_non_string_fields() {
        let input: DeriveInput = syn::parse_quote! {
            #[cfmd(key = "user")]
            struct User {
                #[cfmd(id)] id: Id<User>,
                #[cfmd(matches = ::cfmd::TextPattern::literal("19"))]
                age: i64,
            }
        };
        let error = expand_entity(&input).expect_err("text pattern on i64 must fail");
        assert!(error.to_string().contains("require a String field"));
    }

    #[test]
    fn authoritative_entity_emits_database_definition_capability() {
        let authoritative: DeriveInput = syn::parse_quote! {
            #[cfmd(key = "user", authoritative)]
            struct User {
                #[cfmd(id)] id: Id<User>,
                name: String,
            }
        };
        let reader: DeriveInput = syn::parse_quote! {
            #[cfmd(key = "user")]
            struct ReaderUser {
                #[cfmd(id)] id: Id<ReaderUser>,
                name: String,
            }
        };
        let authoritative = expand_entity(&authoritative)
            .expect("authoritative entity")
            .to_string();
        let reader = expand_entity(&reader).expect("reader entity").to_string();
        assert!(authoritative.contains("AuthoritativeObject"));
        assert!(!reader.contains("AuthoritativeObject"));
    }
}
