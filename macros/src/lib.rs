extern crate proc_macro;

use {
    proc_macro::TokenStream,
    proc_macro2::Span,
    quote::quote,
    syn::{parse_macro_input, DeriveInput},
};
