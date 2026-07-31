//! The formatter deliberately owns this small layout syntax model. A proc-macro
//! crate cannot export its parser, and keeping this copy here leaves the
//! formatter usable without linking Nestix itself.

use proc_macro2::TokenStream;
use syn::{
    Attribute, Expr, FnArg, Ident, Pat, Token, Type, braced, bracketed, parenthesized,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    token,
};

struct Capture {
    _ident: Option<Ident>,
    _expr: Expr,
}

impl Parse for Capture {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident = if input.peek2(Token![:]) {
            let ident = input.parse()?;
            input.parse::<Token![:]>()?;
            Some(ident)
        } else {
            None
        };
        Ok(Self {
            _ident: ident,
            _expr: input.parse()?,
        })
    }
}

struct Element;

impl Parse for Element {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        input.call(Attribute::parse_outer)?;
        if input.peek(Token![yield]) {
            input.parse::<Token![yield]>()?;
        }
        if input.peek2(Token![@]) {
            input.parse::<Ident>()?;
            input.parse::<Token![@]>()?;
        }
        input.parse::<Type>()?;
        if input.peek(Token![$]) {
            input.parse::<Token![$]>()?;
            let inner;
            parenthesized!(inner in input);
            inner.parse::<TokenStream>()?;
        } else if input.peek(token::Paren) {
            let inner;
            parenthesized!(inner in input);
            inner.parse::<TokenStream>()?;
        }
        if input.peek(token::Bracket) {
            let inner;
            bracketed!(inner in input);
            Punctuated::<Capture, Token![,]>::parse_terminated(&inner)?;
        }
        if input.peek(Token![|]) {
            input.parse::<Token![|]>()?;
            let mut args = Punctuated::<FnArg, Token![,]>::new();
            while !input.peek(Token![|]) {
                args.push_value(input.parse()?);
                if input.peek(Token![,]) {
                    args.push_punct(input.parse()?);
                }
            }
            input.parse::<Token![|]>()?;
        }
        if input.peek(token::Brace) {
            let inner;
            braced!(inner in input);
            inner.parse::<Layout>()?;
        }
        Ok(Self)
    }
}

struct If;

impl Parse for If {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        input.parse::<Token![if]>()?;
        Expr::parse_without_eager_brace(input)?;
        let inner;
        braced!(inner in input);
        inner.parse::<Layout>()?;
        if input.peek(Token![else]) {
            input.parse::<Token![else]>()?;
            if input.peek(Token![if]) {
                input.parse::<If>()?;
            } else {
                let inner;
                braced!(inner in input);
                inner.parse::<Layout>()?;
            }
        }
        Ok(Self)
    }
}

struct For;

impl Parse for For {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        input.parse::<Token![for]>()?;
        input.parse::<Ident>()?;
        input.parse::<Token![in]>()?;
        Expr::parse_without_eager_brace(input)?;
        if input.peek(Token![where]) {
            let fork = input.fork();
            fork.parse::<Token![where]>()?;
            if fork.parse::<Ident>()? == "key" {
                input.parse::<Token![where]>()?;
                input.parse::<Ident>()?;
                input.parse::<Token![=]>()?;
                Expr::parse_without_eager_brace(input)?;
            }
        }
        let inner;
        braced!(inner in input);
        inner.parse::<Layout>()?;
        Ok(Self)
    }
}

struct Match;

impl Parse for Match {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        input.parse::<Token![match]>()?;
        Expr::parse_without_eager_brace(input)?;
        let inner;
        braced!(inner in input);
        while !inner.is_empty() {
            Pat::parse_multi_with_leading_vert(&inner)?;
            if inner.peek(Token![if]) {
                inner.parse::<Token![if]>()?;
                Expr::parse_without_eager_brace(&inner)?;
            }
            inner.parse::<Token![=>]>()?;
            let body;
            braced!(body in inner);
            body.parse::<Layout>()?;
            if inner.peek(Token![,]) {
                inner.parse::<Token![,]>()?;
            }
        }
        Ok(Self)
    }
}

pub struct Layout;

impl Parse for Layout {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        while !input.is_empty() {
            if input.peek(Token![$]) || (input.peek(Token![yield]) && input.peek2(Token![$])) {
                if input.peek(Token![yield]) {
                    input.parse::<Token![yield]>()?;
                }
                input.parse::<Token![$]>()?;
                let inner;
                parenthesized!(inner in input);
                Expr::parse_without_eager_brace(&inner)?;
            } else if input.peek(Token![if]) {
                input.parse::<If>()?;
            } else if input.peek(Token![for]) {
                input.parse::<For>()?;
            } else if input.peek(Token![match]) {
                input.parse::<Match>()?;
            } else {
                input.parse::<Element>()?;
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(Self)
    }
}

pub fn validate(source: &str) -> syn::Result<()> {
    syn::parse_str::<Layout>(source).map(|_| ())
}
