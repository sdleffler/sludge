use proc_macro2::Span;
use proc_macro_crate::crate_name;
use quote::quote;
use syn::*;

fn guess_name() -> Ident {
    match crate_name("sludge") {
        Ok(name) => Ident::new(&name, Span::call_site()),
        Err(_) => Ident::new("crate", Span::call_site()),
    }
}

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

    let root = guess_name();

    let expanded = quote! {
        // The generated impl.
        impl #impl_generics #root::ecs::SmartComponent<#root::ecs::ScContext<#context_lifetime>>
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

    let root = guess_name();

    let expanded = quote! {
        // The generated impl.
        impl #impl_generics #root::ecs::SmartComponent<#root::ecs::ScContext<#context_lifetime>>
            for #name #original_generics #where_clause {
            fn on_borrow_mut(&mut self, entity: #root::ecs::Entity, context: #root::ecs::ScContext<#context_lifetime>) {
                context[&#root::TypeId::of::<#name #original_generics>()].emit_modified_atomic(entity);
            }
        }

        // Register the flagged component so that `World`s create a channel for it.
        #root::inventory::submit! {
            #root::ecs::FlaggedComponent::of::<#name #original_generics>()
        }
    };

    // Hand the output tokens back to the compiler.
    proc_macro::TokenStream::from(expanded)
}
