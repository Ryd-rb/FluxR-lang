//! Bounded linking for a bundle of checked, argument-only entry points.
use super::{Expr, Function, Module, Source, SourceDocument, Stmt, error};
use crate::Error;
use std::collections::{BTreeMap, BTreeSet};

impl SourceDocument {
    /// Compile one independently admitted portable module per public function.
    ///
    /// Functions may call free functions in other documents. Only reachable
    /// bodies belong to each entry module. Module limits and host signatures
    /// therefore apply independently to each entry. This profile requires
    /// explicit arguments and excludes methods and native declarations.
    pub fn compile_entry_modules<'a>(
        documents: impl IntoIterator<Item = &'a str>,
    ) -> Result<BTreeMap<String, Module>, Error> {
        let mut combined = Source {
            version: 1,
            inputs: vec![],
            records: vec![],
            enums: vec![],
            functions: vec![],
        };
        let mut graph = BTreeMap::new();
        let mut bytes = 0usize;
        let mut exports = Vec::new();
        for (i, text) in documents.into_iter().enumerate() {
            bytes = bytes.saturating_add(text.len());
            if i >= 256 || bytes > super::unified_source::MAX_SOURCE_BYTES {
                return Err(error("entry bundle source limit"));
            }
            let doc = Self::parse(text).map_err(|e| error(format!("document {}: {e}", i + 1)))?;
            if doc.has_native_items() || !doc.source.inputs.is_empty() {
                return Err(error(
                    "entry bundles require checked argument-only functions",
                ));
            }
            combined.records.extend(doc.source.records);
            combined.enums.extend(doc.source.enums);
            for f in doc.source.functions {
                if combined.functions.len() >= 2048 {
                    return Err(error("entry bundle function limit"));
                }
                if f.receiver.is_some() {
                    return Err(error("entry bundles exclude methods"));
                }
                let mut calls = BTreeSet::new();
                collect_statements(&f.body, &mut calls)?;
                if graph.insert(f.name.clone(), calls).is_some() {
                    return Err(error(format!("duplicate function {}", f.name)));
                }
                if f.export {
                    exports.push(f.name.clone());
                    if exports.len() > 256 {
                        return Err(error("entry bundle export limit"));
                    }
                }
                combined.functions.push(f);
            }
        }
        if exports.is_empty() {
            return Err(error("entry bundle has no public functions"));
        }
        let mut result = BTreeMap::new();
        for name in exports {
            let mut names = BTreeSet::new();
            let mut todo = vec![&name];
            while let Some(current) = todo.pop() {
                if names.insert(current) {
                    todo.extend(
                        graph
                            .get(current)
                            .ok_or_else(|| error(format!("unknown function {current}")))?,
                    )
                }
            }
            let functions = combined
                .functions
                .iter()
                .filter(|f| names.contains(&f.name))
                .map(|f| Function {
                    export: f.name == name,
                    ..f.clone()
                })
                .collect();
            let entry = Source {
                functions,
                version: combined.version,
                inputs: vec![],
                records: combined.records.clone(),
                enums: combined.enums.clone(),
            };
            let module = Module::compile_unified(&entry)
                .map_err(|e| error(format!("linking {name}: {e}")))?;
            result.insert(name, module);
        }
        Ok(result)
    }
}
fn collect_statements(items: &[Stmt], out: &mut BTreeSet<String>) -> Result<(), Error> {
    for item in items {
        match item {
            Stmt::Set { value, .. }
            | Stmt::SetField { value, .. }
            | Stmt::Push { value, .. }
            | Stmt::Return { value } => collect_expr(value, out)?,
            Stmt::If {
                test,
                then,
                otherwise,
            } => {
                collect_expr(test, out)?;
                collect_statements(then, out)?;
                collect_statements(otherwise, out)?;
            }
            Stmt::While { test, body } => {
                collect_expr(test, out)?;
                collect_statements(body, out)?;
            }
            Stmt::Break | Stmt::Continue | Stmt::Pop { .. } => {}
        }
    }
    Ok(())
}
fn collect_expr(expr: &Expr, out: &mut BTreeSet<String>) -> Result<(), Error> {
    match expr {
        Expr::Call { name, args } => {
            out.insert(name.clone());
            for arg in args {
                collect_expr(arg, out)?;
            }
        }
        Expr::HostCall { args, .. }
        | Expr::List { items: args, .. }
        | Expr::Record { fields: args, .. } => {
            for arg in args {
                collect_expr(arg, out)?;
            }
        }
        Expr::Method { .. } => return Err(error("entry bundles exclude method calls")),
        Expr::Binary { left, right, .. }
        | Expr::Index {
            list: left,
            index: right,
        } => {
            collect_expr(left, out)?;
            collect_expr(right, out)?;
        }
        Expr::Unary { value, .. }
        | Expr::Field { record: value, .. }
        | Expr::IsVariant { value, .. }
        | Expr::Payload { value, .. }
        | Expr::Empty { list: value }
        | Expr::Last { list: value }
        | Expr::ArgMinF32 { list: value } => collect_expr(value, out)?,
        Expr::Variant {
            payload: Some(value),
            ..
        } => collect_expr(value, out)?,
        Expr::Variant { payload: None, .. }
        | Expr::Unit
        | Expr::Bool { .. }
        | Expr::I64 { .. }
        | Expr::U64 { .. }
        | Expr::F32 { .. }
        | Expr::Local { .. }
        | Expr::Input { .. } => {}
    }
    Ok(())
}
