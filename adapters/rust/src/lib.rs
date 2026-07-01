// glucore_macro — POST-TASK-3 STATE
//
// The macro:
//   1. Parses each parameter's syn::Type and the return type.
//   2. Matches against a fixed whitelist:
//        f64           -> Float
//        i64           -> Int
//        String        -> String (owned)
//        &str          -> String (borrowed)
//        Vec<u8>       -> Buffer (owned)
//        &[u8]         -> Buffer (borrowed)
//        ()            -> Void
//      Any type not in the whitelist -> compile_error! pointing at the
//      offending type. (Constraint 2.)
//   3. Generates per-parameter unpacking and per-return packing branched on
//      the matched type — including String/Buffer as of Task 3.
//   4. Wraps the native call in std::panic::catch_unwind. (Constraint 3.)
//   5. Emits a `static __SIG_<FN>: GluSignature` and an
//      `inventory::submit!` per function (Task 2).

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{parse_macro_input, ItemFn, Type};

#[proc_macro_attribute]
pub fn export(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let func = parse_macro_input!(item as ItemFn);
    let fn_name = &func.sig.ident;
    let fn_name_str = fn_name.to_string();
    let wrapper_name = syn::Ident::new(&format!("__wrapper_{}", fn_name_str), fn_name.span());
    let sig_name = syn::Ident::new(&format!("__SIG_{}", fn_name_str.to_uppercase()), fn_name.span());

    // --- Parse each parameter's type ---
    let mut param_tags: Vec<Tag> = Vec::new();
    let mut param_unpackers: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut param_ids: Vec<proc_macro2::TokenStream> = Vec::new();

    for (i, input) in func.sig.inputs.iter().enumerate() {
        let pat_type = match input {
            syn::FnArg::Typed(pt) => pt,
            syn::FnArg::Receiver(_) => {
                return syn::Error::new_spanned(input, "glucore::export: self receivers not supported")
                    .to_compile_error()
                    .into();
            }
        };
        let ty = &*pat_type.ty;
        let tag = match match_type(ty) {
            Ok(t) => t,
            Err(msg) => {
                return syn::Error::new_spanned(ty, msg).to_compile_error().into();
            }
        };
        let arg_id = syn::Ident::new(&format!("arg{}", i), Span::call_site());
        let unpacker = generate_unpacker(tag, &arg_id, i);
        param_tags.push(tag);
        param_unpackers.push(unpacker);
        param_ids.push(quote! { #arg_id });
    }

    // --- Parse the return type ---
    let return_tag = match &func.sig.output {
        syn::ReturnType::Default => Tag::Void, // no `-> Type` means `-> ()`
        syn::ReturnType::Type(_, ty) => match match_type(ty) {
            Ok(t) => t,
            Err(msg) => {
                return syn::Error::new_spanned(ty, msg).to_compile_error().into();
            }
        },
    };

    let packer = generate_packer(return_tag);

    // --- Build the GluSignature static ---
    let param_tags_tokens: Vec<proc_macro2::TokenStream> = param_tags
        .iter()
        .map(|t| tag_to_core_variant(*t))
        .collect();
    let return_tag_token = tag_to_core_variant(return_tag);

    // --- Generate the wrapper ---
    let panic_msg = format!("panic in {}", fn_name_str);

    let expanded = quote! {
        // Keep the original function in place.
        #func

        // Per-function signature metadata.
        static #sig_name: glucore_core::GluSignature = glucore_core::GluSignature {
            params: &[#(#param_tags_tokens),*],
            return_type: #return_tag_token,
        };

        #[no_mangle]
        pub extern "C" fn #wrapper_name(
            args: *const glucore_core::GluValue,
            argc: usize,
        ) -> glucore_core::GluResult {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let args = unsafe { std::slice::from_raw_parts(args, argc) };
                #(#param_unpackers)*
                let ret = #fn_name(#(#param_ids),*);
                #packer
            }));
            match outcome {
                Ok(value) => glucore_core::GluResult::ok(value),
                Err(_) => glucore_core::GluResult::err(
                    glucore_core::GluStatus::Runtime,
                    #panic_msg,
                ),
            }
        }

        // Auto-register this export via inventory (Task 2).
        inventory::submit! {
            glucore_core::GluExport {
                name: #fn_name_str,
                wrapper: #wrapper_name,
                signature: #sig_name,
            }
        }
    };

    expanded.into()
}

// --- Type whitelist --------------------------------------------------------

/// Internal tag. Distinguishes owned vs borrowed String/Buffer so the
/// generated unpacker produces the right Rust type (String vs &str, Vec<u8>
/// vs &[u8]). The external GluTypeTag does NOT distinguish — both owned and
/// borrowed map to GluTypeTag::String (or ::Buffer).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tag {
    Int,
    Float,
    StringOwned,    // String
    StringBorrowed, // &str
    BufferOwned,    // Vec<u8>
    BufferBorrowed, // &[u8]
    Void,
}

fn match_type(ty: &Type) -> Result<Tag, String> {
    match ty {
        Type::Path(p) => {
            let seg = p.path.segments.last().ok_or("empty type path")?;
            let ident = seg.ident.to_string();
            match (ident.as_str(), &seg.arguments) {
                ("f64", syn::PathArguments::None) => Ok(Tag::Float),
                ("i64", syn::PathArguments::None) => Ok(Tag::Int),
                ("String", syn::PathArguments::None) => Ok(Tag::StringOwned),
                ("Vec", syn::PathArguments::AngleBracketed(args)) => {
                    let inner = args.args.iter().next().ok_or("Vec<> without type arg")?;
                    if is_u8_arg(inner) {
                        Ok(Tag::BufferOwned)
                    } else {
                        Err(format!(
                            "glucore::export: unsupported Vec<T> type, only Vec<u8> is supported (got Vec<{}>)",
                            arg_to_string(inner)
                        ))
                    }
                }
                _ => Err(format!(
                    "glucore::export: unsupported parameter/return type '{}', expected one of: f64, i64, String, &str, Vec<u8>, &[u8]",
                    type_to_string(ty)
                )),
            }
        }
        Type::Reference(r) => {
            match &*r.elem {
                Type::Path(p) => {
                    let seg = p.path.segments.last().ok_or("empty type path")?;
                    if seg.ident == "str" {
                        Ok(Tag::StringBorrowed)
                    } else {
                        Err(format!(
                            "glucore::export: unsupported reference type '&{}', expected &str or &[u8]",
                            seg.ident
                        ))
                    }
                }
                Type::Slice(s) => {
                    if is_u8_type(&*s.elem) {
                        Ok(Tag::BufferBorrowed)
                    } else {
                        Err(format!(
                            "glucore::export: unsupported slice type '&[{}]', expected &[u8]",
                            type_to_string(&*s.elem)
                        ))
                    }
                }
                _ => Err(format!(
                    "glucore::export: unsupported reference type '{}', expected &str or &[u8]",
                    type_to_string(ty)
                )),
            }
        }
        Type::Tuple(t) if t.elems.is_empty() => Ok(Tag::Void),
        _ => Err(format!(
            "glucore::export: unsupported parameter/return type '{}', expected one of: f64, i64, String, &str, Vec<u8>, &[u8]",
            type_to_string(ty)
        )),
    }
}

fn is_u8_type(ty: &Type) -> bool {
    if let Type::Path(p) = ty {
        if let Some(seg) = p.path.segments.last() {
            return seg.ident == "u8" && matches!(seg.arguments, syn::PathArguments::None);
        }
    }
    false
}

fn is_u8_arg(arg: &syn::GenericArgument) -> bool {
    if let syn::GenericArgument::Type(t) = arg {
        return is_u8_type(t);
    }
    false
}

fn type_to_string(ty: &Type) -> String {
    quote::ToTokens::to_token_stream(ty).to_string().replace(" ", "")
}

fn arg_to_string(arg: &syn::GenericArgument) -> String {
    quote::ToTokens::to_token_stream(arg).to_string().replace(" ", "")
}

// --- Per-type code generation ----------------------------------------------

fn tag_to_core_variant(t: Tag) -> proc_macro2::TokenStream {
    match t {
        Tag::Int => quote! { glucore_core::GluTypeTag::Int },
        Tag::Float => quote! { glucore_core::GluTypeTag::Float },
        Tag::StringOwned | Tag::StringBorrowed => quote! { glucore_core::GluTypeTag::String },
        Tag::BufferOwned | Tag::BufferBorrowed => quote! { glucore_core::GluTypeTag::Buffer },
        Tag::Void => quote! { glucore_core::GluTypeTag::Void },
    }
}

/// Generate `let arg<i> = <unpack GluValue[i] as tag>;`
fn generate_unpacker(tag: Tag, arg_id: &syn::Ident, idx: usize) -> proc_macro2::TokenStream {
    match tag {
        Tag::Float => quote! {
            let #arg_id = unsafe { args[#idx].float };
        },
        Tag::Int => quote! {
            let #arg_id = unsafe { args[#idx].int };
        },
        Tag::StringOwned => quote! {
            // Copy bytes from the GluValue into an owned String. Per
            // Constraint 4: Rust owns its own copy.
            let #arg_id = unsafe {
                let slice = std::slice::from_raw_parts(
                    args[#idx].string.ptr,
                    args[#idx].string.len,
                );
                String::from_utf8_lossy(slice).into_owned()
            };
        },
        Tag::StringBorrowed => quote! {
            // Borrowed &str — valid for the duration of the wrapper call
            // (the GluValue's bytes are owned by Python and live through
            // the call).
            let #arg_id = unsafe {
                let slice = std::slice::from_raw_parts(
                    args[#idx].string.ptr,
                    args[#idx].string.len,
                );
                std::str::from_utf8_unchecked(slice)
            };
        },
        Tag::BufferOwned => quote! {
            let #arg_id = unsafe {
                let slice = std::slice::from_raw_parts(
                    args[#idx].buffer.ptr,
                    args[#idx].buffer.len,
                );
                slice.to_vec()
            };
        },
        Tag::BufferBorrowed => quote! {
            let #arg_id = unsafe {
                std::slice::from_raw_parts(
                    args[#idx].buffer.ptr,
                    args[#idx].buffer.len,
                )
            };
        },
        Tag::Void => quote! {
            // Void parameter — shouldn't happen in a function signature, but
            // be defensive.
            let #arg_id = ();
        },
    }
}

/// Generate code that packs `ret` (the native call's return value) into a
/// GluValue. This is the last expression inside the catch_unwind closure.
///
/// For String/Buffer returns: hand Python an *owned* allocation via
/// `Box::into_raw`, so Python can call `glucore_free_buffer(ptr, len)` to
/// reclaim it. (Task 6a — replaces the unbounded `Box::leak` growth from
/// Phase 1. Constraint 4, copy semantics, still holds: Rust owns the bytes
/// it allocates here, Python owns its own copy after unpacking.)
fn generate_packer(tag: Tag) -> proc_macro2::TokenStream {
    match tag {
        Tag::Float => quote! {
            glucore_core::GluValue { float: ret }
        },
        Tag::Int => quote! {
            glucore_core::GluValue { int: ret }
        },
        Tag::StringOwned => quote! {
            let boxed: Box<[u8]> = ret.into_bytes().into_boxed_slice();
            let len = boxed.len();
            let ptr = Box::into_raw(boxed) as *mut u8;
            glucore_core::GluValue {
                string: glucore_core::GluSlice { ptr, len },
            }
        },
        Tag::StringBorrowed => quote! {
            // &str return — copy into an owned allocation handed to Python.
            let boxed: Box<[u8]> = ret.to_string().into_bytes().into_boxed_slice();
            let len = boxed.len();
            let ptr = Box::into_raw(boxed) as *mut u8;
            glucore_core::GluValue {
                string: glucore_core::GluSlice { ptr, len },
            }
        },
        Tag::BufferOwned => quote! {
            let boxed: Box<[u8]> = ret.into_boxed_slice();
            let len = boxed.len();
            let ptr = Box::into_raw(boxed) as *mut u8;
            glucore_core::GluValue {
                buffer: glucore_core::GluSlice { ptr, len },
            }
        },
        Tag::BufferBorrowed => quote! {
            let boxed: Box<[u8]> = ret.to_vec().into_boxed_slice();
            let len = boxed.len();
            let ptr = Box::into_raw(boxed) as *mut u8;
            glucore_core::GluValue {
                buffer: glucore_core::GluSlice { ptr, len },
            }
        },
        Tag::Void => quote! {
            { let _ = ret; glucore_core::GluValue { int: 0 } }
        },
    }
}
