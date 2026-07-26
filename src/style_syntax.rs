use proc_macro2::TokenStream;
use syn::{
    Expr, Ident, Token, braced, bracketed, parenthesized,
    parse::{Parse, ParseStream},
    token,
};

struct StyleSheet;

impl Parse for StyleSheet {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        while !input.is_empty() {
            if input.peek(Token![$]) {
                input.parse::<Token![$]>()?;
                let inner;
                parenthesized!(inner in input);
                inner.parse::<Expr>()?;
            } else {
                input.parse::<StyleRule>()?;
            }
        }
        Ok(Self)
    }
}

struct StyleRule;

impl Parse for StyleRule {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        input.parse::<SelectorList>()?;
        let inner;
        braced!(inner in input);
        while !inner.is_empty() {
            if inner.peek(Ident) || inner.peek(Token![-]) {
                inner.parse::<StyleProperty>()?;
            } else {
                inner.parse::<StyleRule>()?;
            }
        }
        Ok(Self)
    }
}

struct SelectorList;

impl Parse for SelectorList {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        loop {
            input.parse::<SelectorChain>()?;
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else {
                break;
            }
        }
        Ok(Self)
    }
}

struct SelectorChain;

impl Parse for SelectorChain {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.peek(Token![>]) || input.peek(Token![+]) || input.peek(Token![~]) {
            input.parse::<proc_macro2::Punct>()?;
            if input.peek(Token![>]) {
                input.parse::<Token![>]>()?;
            }
        }
        input.parse::<CompoundSelector>()?;
        while input.peek(Token![>]) || input.peek(Token![+]) || input.peek(Token![~]) {
            input.parse::<proc_macro2::Punct>()?;
            if input.peek(Token![>]) {
                input.parse::<Token![>]>()?;
            }
            input.parse::<CompoundSelector>()?;
        }
        Ok(Self)
    }
}

struct CompoundSelector;

impl Parse for CompoundSelector {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut found = false;
        while input.peek(Token![.]) || input.peek(Token![:]) || input.peek(Token![&]) {
            found = true;
            if input.peek(Token![.]) {
                input.parse::<Token![.]>()?;
                input.parse::<Ident>()?;
            } else if input.peek(Token![&]) {
                input.parse::<Token![&]>()?;
            } else {
                input.parse::<Token![:]>()?;
                let name: Ident = input.parse()?;
                match name.to_string().as_str() {
                    "not" => {
                        let inner;
                        parenthesized!(inner in input);
                        inner.parse::<SelectorList>()?;
                    }
                    "nth_child" => {
                        let inner;
                        parenthesized!(inner in input);
                        if inner.is_empty() {
                            return Err(
                                inner.error("expected an `An+B` expression in `nth_child()`")
                            );
                        }
                        inner.parse::<TokenStream>()?;
                    }
                    "first_child" | "last_child" => {}
                    _ => {
                        return Err(syn::Error::new_spanned(
                            name,
                            "unsupported pseudo selector; expected `not`, `first_child`, `last_child`, or `nth_child`",
                        ));
                    }
                }
            }
        }
        if found {
            Ok(Self)
        } else {
            Err(input.error("expected selector"))
        }
    }
}

struct StyleProperty;

impl Parse for StyleProperty {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.peek(Token![-]) {
            input.parse::<Token![-]>()?;
            input.parse::<Token![-]>()?;
        }
        input.parse::<Ident>()?;
        input.parse::<Token![:]>()?;
        if input.peek(Token![$]) {
            input.parse::<Token![$]>()?;
            let inner;
            parenthesized!(inner in input);
            inner.parse::<Expr>()?;
        } else {
            while !input.peek(Token![;]) {
                if input.is_empty() {
                    return Err(input.error("expected `;` after style property value"));
                }
                input.parse::<proc_macro2::TokenTree>()?;
            }
        }
        input.parse::<Token![;]>()?;
        Ok(Self)
    }
}

pub fn validate(source: &str, computed: bool) -> syn::Result<()> {
    if computed {
        syn::parse::Parser::parse_str(
            |input: ParseStream| {
                if input.peek(token::Bracket) {
                    let captures;
                    bracketed!(captures in input);
                    captures.parse::<TokenStream>()?;
                }
                input.parse::<StyleSheet>()
            },
            source,
        )
        .map(|_| ())
    } else {
        syn::parse_str::<StyleSheet>(source).map(|_| ())
    }
}
