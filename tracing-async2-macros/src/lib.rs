mod macros;

use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn tracing_layer_async_within(args: TokenStream, item: TokenStream) -> TokenStream {
    macros::build_layer_function(args, item)
}
