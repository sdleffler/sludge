use proc_macro2::Span;
use quote::quote;
use syn::*;

#[proc_macro_derive(SimpleComponent)]
pub fn derive_simple_component(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    // Parse the input tokens into a syntax tree.
    let input = parse_macro_input!(input as DeriveInput);

    // Used in the quasi-quotation below as `#name`.
    let name = input.ident;

    let context_lifetime = Lifetime::new("'a", Span::call_site());
    let mut generics = input.generics.clone();
    generics.params.insert(
        0,
        GenericParam::Lifetime(LifetimeDef::new(context_lifetime.clone())),
    );
    let (impl_generics, _, where_clause) = generics.split_for_impl();
    let (_, ty_generics, _) = input.generics.split_for_impl();

    let expanded = quote! {
        // The generated impl.
        impl #impl_generics ::sludge::ecs::SmartComponent<::sludge::ecs::ScContext<#context_lifetime>>
            for #name #ty_generics #where_clause {}
    };

    // Hand the output tokens back to the compiler.
    proc_macro::TokenStream::from(expanded)
}

#[proc_macro_derive(FlaggedComponent)]
pub fn derive_flagged_component(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    // Parse the input tokens into a syntax tree.
    let input = parse_macro_input!(input as DeriveInput);

    // Used in the quasi-quotation below as `#name`.
    let name = input.ident;

    let context_lifetime = Lifetime::new("'a", Span::call_site());
    let mut generics = input.generics.clone();
    generics.params.insert(
        0,
        GenericParam::Lifetime(LifetimeDef::new(context_lifetime.clone())),
    );
    let (impl_generics, _, where_clause) = generics.split_for_impl();
    let (_, original_generics, _) = input.generics.split_for_impl();

    let expanded = quote! {
        // The generated impl.
        impl #impl_generics ::sludge::SmartComponent<::sludge::ScContext<#context_lifetime>>
            for #name #original_generics #where_clause {
            fn on_borrow_mut(&self, entity: ::sludge::Entity, context: ::sludge::ScContext<#context_lifetime>) {
                context[&::sludge::TypeId::of::<#name #original_generics>()].set_modified_atomic(entity.id());
            }
        }

        // Register the flagged component so that `World`s create a channel for it.
        ::sludge::inventory::submit! {
            ::sludge::FlaggedComponent::of::<Parent>()
        }
    };

    // Hand the output tokens back to the compiler.
    proc_macro::TokenStream::from(expanded)
}
