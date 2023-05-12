use darling::FromMeta;
use proc_macro::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::{parse_macro_input, AttributeArgs};

#[derive(Debug, FromMeta)]
struct PlcProgramArgs {
    #[darling(rename = "loop")]
    lp: String,
}

/// # Panics
///
/// Will panic if function name is more than 14 symbols
#[proc_macro_attribute]
pub fn plc_program(args: TokenStream, input: TokenStream) -> TokenStream {
    let attr_args = parse_macro_input!(args as AttributeArgs);
    let args = match PlcProgramArgs::from_list(&attr_args) {
        Ok(v) => v,
        Err(e) => {
            return TokenStream::from(e.write_errors());
        }
    };
    let item: syn::Item = syn::parse(input).expect("Invalid input");
    let int = parse_interval(&args.lp).unwrap();
    if let syn::Item::Fn(fn_item) = item {
        let block = fn_item.block;
        let name = fn_item.sig.ident;
        assert!(
            name.to_string().len() < 15,
            "function name must be less than 15 symbols ({})",
            name
        );
        assert!(
            fn_item.sig.inputs.is_empty(),
            "function must have no arguments ({})",
            name
        );
        assert!(
            fn_item.sig.output == syn::ReturnType::Default,
            "function must not return a value ({})",
            name
        );
        let spawner_name = format_ident!("{}_spawn", name);
        let prgname = name.to_string();
        let f = quote! {
            fn #spawner_name() {
                ::rplc::tasks::spawn_program_loop(#prgname,
                    #name,
                    ::std::time::Duration::from_nanos(#int));
            }
            fn #name() {
                #block
            }
        };
        f.into_token_stream().into()
    } else {
        panic!("expected fn")
    }
}

#[derive(Debug)]
enum PError {
    Parse,
}

fn parse_interval(s: &str) -> Result<u64, PError> {
    if let Some(v) = s.strip_suffix("ms") {
        Ok(v.parse::<u64>().map_err(|_| PError::Parse)? * 1_000_000)
    } else if let Some(v) = s.strip_suffix("us") {
        Ok(v.parse::<u64>().map_err(|_| PError::Parse)? * 1_000)
    } else if let Some(v) = s.strip_suffix("ns") {
        Ok(v.parse::<u64>().map_err(|_| PError::Parse)?)
    } else if let Some(v) = s.strip_suffix('s') {
        Ok(v.parse::<u64>().map_err(|_| PError::Parse)? * 1_000_000_000)
    } else {
        Err(PError::Parse)
    }
}
