//! Lower integer ranges before Rust-GPU legalization. `Range::next` carries
//! unchecked-step assertions that force even unrelated SDF callers to inline.
use proc_macro2::{Span, TokenStream, TokenTree};
use quote::{ToTokens, format_ident};
use std::collections::HashSet;
use syn::{
    Expr, File, Lit, RangeLimits, parse_quote,
    visit_mut::{self, VisitMut},
};

pub(super) fn lower(file: &mut File) {
    fn identifiers(tokens: TokenStream, output: &mut HashSet<String>) {
        for token in tokens {
            match token {
                TokenTree::Ident(name) => {
                    output.insert(name.to_string());
                }
                TokenTree::Group(group) => identifiers(group.stream(), output),
                _ => {}
            }
        }
    }
    let mut names = HashSet::new();
    identifiers(file.to_token_stream(), &mut names);
    Ranges { next: 0, names }.visit_file_mut(file);
}

struct Ranges {
    next: usize,
    names: HashSet<String>,
}

fn integer_literal(expr: &Expr) -> bool {
    match expr {
        Expr::Lit(value) => matches!(value.lit, Lit::Int(_)),
        Expr::Unary(value) => matches!(value.op, syn::UnOp::Neg(_)) && integer_literal(&value.expr),
        _ => false,
    }
}

impl VisitMut for Ranges {
    fn visit_expr_mut(&mut self, i: &mut Expr) {
        visit_mut::visit_expr_mut(self, i);
        let Expr::ForLoop(loop_) = i else {
            return;
        };
        let Expr::Range(range) = &*loop_.expr else {
            return;
        };
        let (Some(start), Some(end)) = (&range.start, &range.end) else {
            return;
        };
        // Integer literals constrain the range to builtin integer types. Leave
        // char/custom iterator ranges and non-range iterators untouched.
        if !integer_literal(start) && !integer_literal(end) {
            return;
        }
        let (cursor, limit, more) = loop {
            let id = self.next;
            self.next += 1;
            let names = [
                format!("__isthmus_range_cursor_{id}"),
                format!("__isthmus_range_end_{id}"),
                format!("__isthmus_range_more_{id}"),
            ];
            if names.iter().all(|name| !self.names.contains(name)) {
                break (
                    format_ident!("{}", names[0], span = Span::mixed_site()),
                    format_ident!("{}", names[1], span = Span::mixed_site()),
                    format_ident!("{}", names[2], span = Span::mixed_site()),
                );
            }
        };
        let (pattern, body, label, attrs) = (&loop_.pat, &loop_.body, &loop_.label, &loop_.attrs);
        // Advance before entering the body, just like Iterator::next, so
        // continue and labelled continue cannot skip the increment.
        *i = match range.limits {
            RangeLimits::HalfOpen(_) => parse_quote!({
                let mut #cursor = #start;
                let #limit = #end;
                #(#attrs)* #label while #cursor < #limit {
                    let #pattern = #cursor;
                    #cursor += 1;
                    #body
                }
            }),
            RangeLimits::Closed(_) => parse_quote!({
                let mut #cursor = #start;
                let #limit = #end;
                let mut #more = #cursor <= #limit;
                #(#attrs)* #label while #more {
                    let #pattern = #cursor;
                    if #cursor == #limit { #more = false; } else { #cursor += 1; }
                    #body
                }
            }),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env, fs,
        process::{self, Command},
    };

    #[test]
    fn lowered_ranges_preserve_bounds_side_effects_labels_continues_and_integer_limits() {
        let original: File = syn::parse_quote! {
            pub fn scenario(start: i16, end: i16) -> (Vec<i16>, usize) {
                let mut values = Vec::new();
                let mut evaluations = 0;
                let __isthmus_range_cursor_0 = 91;
                'outer: for row in 0..({ evaluations += 1; 6 }) {
                    for column in 0..5 {
                        if column == 1 { continue; }
                        if row == 4 { break 'outer; }
                        if row == 2 { continue 'outer; }
                        values.push(row * 10 + column);
                    }
                }
                for value in -3..=end { values.push(value); }
                for value in start..4 { values.push(value); }
                for value in 250u8..=255u8 { values.push(i16::from(value)); }
                for value in 250u8..255u8 { values.push(i16::from(value)); }
                for value in 'a'..='c' { values.push(value as i16); }
                for value in 0..0 { values.push(value); }
                values.push(__isthmus_range_cursor_0);
                (values, evaluations)
            }
        };
        let mut lowered = original.clone();
        lower(&mut lowered);
        let source = quote::quote! {
            mod original { #original }
            mod lowered { #lowered }
            fn main() {
                for start in [-8, -3, 0, 4, 8] {
                    for end in [-8, -3, 0, 4, 8] {
                        assert_eq!(original::scenario(start, end), lowered::scenario(start, end));
                    }
                }
            }
        };
        let directory = env::temp_dir().join(format!("isthmus-range-lowering-{}", process::id()));
        fs::create_dir_all(&directory).unwrap();
        let input = directory.join("main.rs");
        let executable = directory.join("check");
        fs::write(&input, source.to_string()).unwrap();
        let compiled =
            Command::new("rustc").arg("--edition=2024").arg(&input).arg("-o").arg(&executable).output().unwrap();
        assert!(
            compiled.status.success(),
            "generated source must compile: {}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let output = Command::new(&executable).output().unwrap();
        fs::remove_dir_all(directory).unwrap();
        assert!(output.status.success(), "loop behaviour changed: {}", String::from_utf8_lossy(&output.stderr));
    }
}
