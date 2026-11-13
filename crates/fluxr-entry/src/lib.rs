//! Entry instrumentation over Rust token trees, never source-text replacement.
//!
//! This macro is an implementation detail of the single FluxR builder. Foreign
//! Rust remains trusted native code; this is not a sandbox for malicious macros.
use proc_macro::{Delimiter, Group, Ident, Span, TokenStream, TokenTree};
use std::str::FromStr;

fn error(message: &str) -> TokenStream {
    TokenStream::from_str(&format!("compile_error!({message:?});")).unwrap()
}

fn semicolon(token: Option<TokenTree>) -> Result<(), &'static str> {
    match token {
        Some(TokenTree::Punct(p)) if p.as_char() == ';' => Ok(()),
        _ => Err("FluxR program metadata requires a semicolon"),
    }
}

fn group(token: Option<TokenTree>, delimiter: Delimiter) -> Result<Group, &'static str> {
    match token {
        Some(TokenTree::Group(g)) if g.delimiter() == delimiter => Ok(g),
        _ => Err("invalid FluxR program metadata group"),
    }
}

fn is_ident(token: Option<&TokenTree>, name: &str) -> bool {
    matches!(token, Some(TokenTree::Ident(i)) if i.to_string().trim_start_matches("r#") == name)
}

fn startup(sources: &Group, initializer: &Group) -> TokenStream {
    let prefix =
        "let __fluxr_runtime_guard = match ::fluxr::runtime::start_embedded_with_sources(&";
    let suffix = ") { Ok(guard) => guard, Err(error) => { ::std::eprintln!(\"FluxR startup: {}\", error); ::std::process::exit(70); } };";
    // Construct one balanced stream, then restore the source tokens' spans.
    let result = TokenStream::from_str(&format!("{prefix}[]{suffix}")).unwrap();
    let mut tokens: Vec<TokenTree> = result.into_iter().collect();
    fn replace_array(tokens: &mut [TokenTree], source: &Group) -> bool {
        for token in tokens {
            if let TokenTree::Group(g) = token {
                if g.delimiter() == Delimiter::Bracket && g.stream().is_empty() {
                    *g = source.clone();
                    return true;
                }
                let mut inner: Vec<_> = g.stream().into_iter().collect();
                if replace_array(&mut inner, source) {
                    let mut replacement = Group::new(g.delimiter(), inner.into_iter().collect());
                    replacement.set_span(g.span());
                    *g = replacement;
                    return true;
                }
            }
        }
        false
    }
    assert!(replace_array(&mut tokens, sources));
    // Mixed-site hygiene prevents a user local with this spelling from
    // shadowing/dropping the runtime guard before the application returns.
    for token in &mut tokens {
        if let TokenTree::Ident(i) = token
            && i.to_string() == "__fluxr_runtime_guard"
        {
            *i = Ident::new("__fluxr_runtime_guard", Span::mixed_site());
        }
    }
    let mut result: TokenStream = tokens.into_iter().collect();
    result.extend(initializer.stream());
    result
}

fn instrument(
    items: TokenStream,
    sources: &Group,
    initializer: &Group,
    role: &str,
) -> Result<TokenStream, &'static str> {
    let mut tokens: Vec<_> = items.into_iter().collect();
    let mut mains = 0;
    let mut index = 0;
    while index < tokens.len() {
        if is_ident(tokens.get(index), "fn") && is_ident(tokens.get(index + 1), "main") {
            if role == "module" {
                // A normal helper named main in a library or child module is
                // not the process entry. The builder derives roots from Cargo.
                index += 1;
                continue;
            }
            let Some(TokenTree::Group(parameters)) = tokens.get(index + 2) else {
                return Err("generic or malformed main entry is unsupported");
            };
            if parameters.delimiter() != Delimiter::Parenthesis || !parameters.stream().is_empty() {
                return Err("FluxR main must have no parameters");
            }
            let mut body_index = index + 3;
            let mut generic_depth: usize = 0;
            while body_index < tokens.len() {
                match &tokens[body_index] {
                    TokenTree::Group(g)
                        if g.delimiter() == Delimiter::Brace && generic_depth == 0 =>
                    {
                        break;
                    }
                    TokenTree::Punct(p) if p.as_char() == '<' => {
                        generic_depth += 1;
                        body_index += 1;
                    }
                    TokenTree::Punct(p) if p.as_char() == '>' => {
                        generic_depth = generic_depth.saturating_sub(1);
                        body_index += 1;
                    }
                    TokenTree::Punct(p) if p.as_char() == ';' => {
                        return Err("main entry requires a body");
                    }
                    _ => body_index += 1,
                }
            }
            let Some(TokenTree::Group(body)) = tokens.get(body_index) else {
                return Err("main entry requires a body");
            };
            let original: Vec<_> = body.stream().into_iter().collect();
            let mut inner_end = 0;
            while matches!(original.get(inner_end), Some(TokenTree::Punct(p)) if p.as_char()=='#')
                && matches!(original.get(inner_end + 1), Some(TokenTree::Punct(p)) if p.as_char()=='!')
                && matches!(original.get(inner_end + 2), Some(TokenTree::Group(g)) if g.delimiter()==Delimiter::Bracket)
            {
                inner_end += 3;
            }
            let mut content: TokenStream = original[..inner_end].iter().cloned().collect();
            content.extend(startup(sources, initializer));
            content.extend(original[inner_end..].iter().cloned());
            let mut replacement = Group::new(Delimiter::Brace, content);
            replacement.set_span(body.span());
            tokens[body_index] = TokenTree::Group(replacement);
            mains += 1;
            index = body_index + 1;
        } else {
            index += 1;
        }
    }
    if role == "binary" && mains == 0 {
        return Err(
            "FluxR binary requires an explicit main; main hidden in include!/macros is unsupported",
        );
    }
    let mut output: TokenStream = tokens.into_iter().collect();
    if role == "test" {
        // Rust's generated test-harness entry does not call the user's main.
        // Include one explicit runtime admission test in this testing-only route.
        let body = startup(sources, initializer);
        let template = format!("#[test] fn __fluxr_unified_runtime_is_embedded() {{ {body} }}");
        output.extend(TokenStream::from_str(&template).unwrap());
    }
    Ok(output)
}

#[proc_macro]
pub fn program(input: TokenStream) -> TokenStream {
    let result = (|| {
        let mut input = input.into_iter();
        let role = match input.next() {
            Some(TokenTree::Ident(i)) => i.to_string(),
            _ => return Err("FluxR program requires a build role"),
        };
        if !matches!(role.as_str(), "binary" | "module" | "test") {
            return Err("invalid FluxR program build role");
        }
        semicolon(input.next())?;
        let sources = group(input.next(), Delimiter::Bracket)?;
        semicolon(input.next())?;
        let initializer = group(input.next(), Delimiter::Brace)?;
        semicolon(input.next())?;
        let items = group(input.next(), Delimiter::Brace)?;
        if input.next().is_some() {
            return Err("trailing FluxR program metadata");
        }
        instrument(items.stream(), &sources, &initializer, &role)
    })();
    result.unwrap_or_else(error)
}
