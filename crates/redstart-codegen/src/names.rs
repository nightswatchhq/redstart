//! Project-wide symbol naming, and the ABIs a project actually binds.
//!
//! Two facts can only be decided by looking at *every* declaration at once, so
//! neither belongs on a single `HandlerDecl`:
//!
//! - **Export names.** `handle{Event}` reads exactly like a hand-written
//!   subgraph, right up to the moment two sources handle the same event name
//!   (three `AddressesRegistry` contracts all emitting
//!   `CollSurplusPoolAddressChanged`, say). Then it emits three functions with
//!   one name, three imports with one alias, and three manifest entries pointing
//!   at it — output `graph build` rejects with `Duplicate identifier`. Where a
//!   name would collide, and only there, the source qualifies it.
//! - **Bound ABIs.** An ABI reached only through `Abi.bind(addr)` — reading
//!   `symbol()` off a token the data source doesn't itself watch — still needs a
//!   generated contract class, which means it must appear in that data source's
//!   `abis:` list. Only a walk of every handler and helper body can find those.

use redstart_parser::ast::{Block, Expr, FnDecl, HandlerDecl, HandlerKind, Stmt};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Resolved, project-wide names for every handler.
///
/// Built once from the full handler list, then shared by the manifest and the
/// mappings so a handler is named identically in both — the manifest's
/// `handler:` entry *is* the exported function.
#[derive(Debug, Default)]
pub struct Names {
    /// Handler key -> exported AssemblyScript function name.
    fns: HashMap<String, String>,
    /// Handler key -> the local alias of its trigger class (event/call handlers).
    triggers: HashMap<String, String>,
}

/// A stable identity for a handler declaration: its source, kind, and member.
fn key(handler: &HandlerDecl) -> String {
    let kind = match handler.kind {
        HandlerKind::Event => 'e',
        HandlerKind::Call => 'c',
        HandlerKind::Block(_) => 'b',
        HandlerKind::File => 'f',
    };
    format!("{}|{kind}|{}", handler.source.name, handler.event.name)
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |c| {
        c.to_uppercase().collect::<String>() + chars.as_str()
    })
}

impl Names {
    /// Resolve export names and trigger aliases across the whole project.
    #[must_use]
    pub fn new(handlers: &[&HandlerDecl]) -> Self {
        // Which unqualified names are claimed more than once? A trigger alias is
        // ambiguous only when *different sources* claim it — two handlers on one
        // source sharing a trigger would be a duplicate declaration, not a
        // collision, and must not qualify itself.
        let mut fn_uses: HashMap<String, usize> = HashMap::new();
        let mut trigger_sources: HashMap<String, HashSet<String>> = HashMap::new();
        for handler in handlers {
            *fn_uses.entry(handler.fn_name()).or_default() += 1;
            if let Some(t) = plain_trigger(handler) {
                trigger_sources
                    .entry(t)
                    .or_default()
                    .insert(handler.source.name.clone());
            }
        }

        let mut fns = HashMap::new();
        let mut triggers = HashMap::new();
        for handler in handlers {
            let plain = handler.fn_name();
            let name = if fn_uses.get(&plain).copied().unwrap_or(0) > 1 {
                qualified_fn_name(handler)
            } else {
                plain
            };
            fns.insert(key(handler), name);

            if let Some(plain) = plain_trigger(handler) {
                let ambiguous = trigger_sources
                    .get(&plain)
                    .is_some_and(|sources| sources.len() > 1);
                let alias = if ambiguous {
                    format!("{}{plain}", capitalize(&handler.source.name))
                } else {
                    plain
                };
                triggers.insert(key(handler), alias);
            }
        }
        Self { fns, triggers }
    }

    /// The exported AssemblyScript function name for a handler.
    #[must_use]
    pub fn fn_name(&self, handler: &HandlerDecl) -> String {
        self.fns
            .get(&key(handler))
            .cloned()
            .unwrap_or_else(|| handler.fn_name())
    }

    /// The local name of a handler's trigger class — the event class for an
    /// event handler, the `<Fn>Call` class for a call handler.
    #[must_use]
    pub fn trigger(&self, handler: &HandlerDecl) -> Option<String> {
        self.triggers.get(&key(handler)).cloned()
    }
}

/// The unqualified local name of a handler's trigger class, if it has one.
fn plain_trigger(handler: &HandlerDecl) -> Option<String> {
    match handler.kind {
        // `Transfer as TransferEvent` — aliased so an entity of the same name
        // can coexist.
        HandlerKind::Event => Some(format!("{}Event", handler.event.name)),
        HandlerKind::Call => Some(format!("{}Call", capitalize(&handler.event.name))),
        HandlerKind::Block(_) | HandlerKind::File => None,
    }
}

/// The source-qualified export name, used only where the plain one collides.
fn qualified_fn_name(handler: &HandlerDecl) -> String {
    let source = capitalize(&handler.source.name);
    match handler.kind {
        HandlerKind::Event => format!("handle{source}{}", capitalize(&handler.event.name)),
        HandlerKind::Call => format!("handle{source}{}Call", capitalize(&handler.event.name)),
        // These already carry the source name.
        HandlerKind::Block(_) | HandlerKind::File => handler.fn_name(),
    }
}

/// Every ABI name the project binds for a contract read (`Abi.bind(addr)`),
/// anywhere in a handler or helper body — including nested in a `match` arm, a
/// loop, or a helper called from a helper.
///
/// `graph codegen` writes one contract class per ABI *listed on a data source*,
/// so an ABI bound but not listed compiles to `Cannot find name 'Abi'`. This is
/// what lets a bind be resolved without declaring an inert template for it.
#[must_use]
pub fn bound_abis(
    handlers: &[&HandlerDecl],
    helpers: &[&FnDecl],
    known: &HashMap<String, std::path::PathBuf>,
) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for handler in handlers {
        walk_block(&handler.body, known, &mut found);
    }
    for helper in helpers {
        walk_block(&helper.body, known, &mut found);
    }
    found
}

fn walk_block(
    block: &Block,
    known: &HashMap<String, std::path::PathBuf>,
    found: &mut BTreeSet<String>,
) {
    for stmt in &block.stmts {
        match stmt {
            Stmt::Let { value, .. } => walk_expr(value, known, found),
            Stmt::Assign { target, value, .. } => {
                walk_expr(target, known, found);
                walk_expr(value, known, found);
            }
            Stmt::Return { value, .. } => {
                if let Some(v) = value {
                    walk_expr(v, known, found);
                }
            }
            Stmt::If {
                cond,
                then_block,
                else_ifs,
                else_block,
                ..
            } => {
                walk_expr(cond, known, found);
                walk_block(then_block, known, found);
                for (c, b) in else_ifs {
                    walk_expr(c, known, found);
                    walk_block(b, known, found);
                }
                if let Some(b) = else_block {
                    walk_block(b, known, found);
                }
            }
            Stmt::While { cond, body, .. } => {
                walk_expr(cond, known, found);
                walk_block(body, known, found);
            }
            Stmt::For { iter, body, .. } => {
                match iter {
                    redstart_parser::ast::ForIter::Range { start, end } => {
                        walk_expr(start, known, found);
                        walk_expr(end, known, found);
                    }
                    redstart_parser::ast::ForIter::Each(e) => walk_expr(e, known, found),
                }
                walk_block(body, known, found);
            }
            Stmt::Expr(e) => walk_expr(e, known, found),
        }
    }
}

fn walk_expr(
    expr: &Expr,
    known: &HashMap<String, std::path::PathBuf>,
    found: &mut BTreeSet<String>,
) {
    match expr {
        Expr::Call { callee, args, .. } => {
            if let Expr::Field { base, field, .. } = callee.as_ref() {
                if field.name == "bind" {
                    if let Expr::Path { segments, .. } = base.as_ref() {
                        if let Some(name) = segments.last() {
                            if known.contains_key(&name.name) {
                                found.insert(name.name.clone());
                            }
                        }
                    }
                }
            }
            walk_expr(callee, known, found);
            for a in args {
                walk_expr(a, known, found);
            }
        }
        Expr::Field { base, .. } => walk_expr(base, known, found),
        Expr::Record { fields, .. } => {
            for (_, v) in fields {
                walk_expr(v, known, found);
            }
        }
        Expr::Array { elems, .. } => {
            for e in elems {
                walk_expr(e, known, found);
            }
        }
        Expr::Index { base, index, .. } => {
            walk_expr(base, known, found);
            walk_expr(index, known, found);
        }
        Expr::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, known, found);
            walk_expr(rhs, known, found);
        }
        Expr::Unary { expr, .. } => walk_expr(expr, known, found),
        Expr::Match {
            scrutinee, arms, ..
        } => {
            walk_expr(scrutinee, known, found);
            for arm in arms {
                walk_block(&arm.body, known, found);
            }
        }
        Expr::Int { .. }
        | Expr::Hex { .. }
        | Expr::Decimal { .. }
        | Expr::Str { .. }
        | Expr::Bool { .. }
        | Expr::Path { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redstart_parser::{lex, parse};
    use std::path::PathBuf;

    fn program(src: &str) -> redstart_parser::ast::Program {
        let lexed = lex(src).expect("lex");
        let (program, errs) = parse(lexed.tokens(), std::sync::Arc::from(src));
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        program
    }

    #[test]
    fn plain_names_survive_when_unambiguous() {
        let p = program(
            "source A { abi: X network: mainnet address: 0x01 startBlock: 1 }
             handler on A.Transfer(event) { }",
        );
        let handlers: Vec<&HandlerDecl> = p.handlers.iter().collect();
        let names = Names::new(&handlers);
        assert_eq!(names.fn_name(handlers[0]), "handleTransfer");
        assert_eq!(names.trigger(handlers[0]).unwrap(), "TransferEvent");
    }

    #[test]
    fn colliding_event_names_are_source_qualified() {
        let p = program(
            "source A { abi: X network: mainnet address: 0x01 startBlock: 1 }
             source b { abi: X network: mainnet address: 0x02 startBlock: 1 }
             handler on A.Transfer(event) { }
             handler on b.Transfer(event) { }",
        );
        let handlers: Vec<&HandlerDecl> = p.handlers.iter().collect();
        let names = Names::new(&handlers);
        assert_eq!(names.fn_name(handlers[0]), "handleATransfer");
        assert_eq!(names.fn_name(handlers[1]), "handleBTransfer");
        // …and their imported trigger classes must not share an alias either.
        assert_eq!(names.trigger(handlers[0]).unwrap(), "ATransferEvent");
        assert_eq!(names.trigger(handlers[1]).unwrap(), "BTransferEvent");
    }

    #[test]
    fn distinct_events_on_one_source_stay_plain() {
        let p = program(
            "source A { abi: X network: mainnet address: 0x01 startBlock: 1 }
             handler on A.Transfer(event) { }
             handler on A.Approval(event) { }",
        );
        let handlers: Vec<&HandlerDecl> = p.handlers.iter().collect();
        let names = Names::new(&handlers);
        assert_eq!(names.fn_name(handlers[0]), "handleTransfer");
        assert_eq!(names.fn_name(handlers[1]), "handleApproval");
    }

    #[test]
    fn colliding_call_handlers_are_source_qualified() {
        let p = program(
            "source A { abi: X network: mainnet address: 0x01 startBlock: 1 }
             source B { abi: X network: mainnet address: 0x02 startBlock: 1 }
             handler call A.swap(call) { }
             handler call B.swap(call) { }",
        );
        let handlers: Vec<&HandlerDecl> = p.handlers.iter().collect();
        let names = Names::new(&handlers);
        assert_eq!(names.fn_name(handlers[0]), "handleASwapCall");
        assert_eq!(names.fn_name(handlers[1]), "handleBSwapCall");
        assert_eq!(names.trigger(handlers[0]).unwrap(), "ASwapCall");
    }

    #[test]
    fn finds_binds_in_handlers_helpers_and_nested_scopes() {
        let p = program(
            r#"source A { abi: X network: mainnet address: 0x01 startBlock: 1 }
               fn readIt(a: Bytes) -> BigInt {
                 let r = Helper.bind(a).total()
                 return BigInt.zero
               }
               handler on A.Transfer(event) {
                 for i in 0..2 {
                   let c = ERC20.bind(event.address).balanceOf(event.params.to)
                   match c { Ok(v) => { let d = Other.bind(event.address).x() } Err(e) => { } }
                 }
               }"#,
        );
        let mut known = HashMap::new();
        for name in ["ERC20", "Helper", "Other", "Unused"] {
            known.insert(name.to_string(), PathBuf::from("x.json"));
        }
        let handlers: Vec<&HandlerDecl> = p.handlers.iter().collect();
        let helpers: Vec<&FnDecl> = p.functions.iter().collect();
        let bound = bound_abis(&handlers, &helpers, &known);
        assert_eq!(
            bound.into_iter().collect::<Vec<_>>(),
            vec!["ERC20", "Helper", "Other"]
        );
    }

    #[test]
    fn a_bind_of_an_undeclared_name_is_not_an_abi() {
        let p = program(
            "source A { abi: X network: mainnet address: 0x01 startBlock: 1 }
             handler on A.Transfer(event) { let x = NotAnAbi.bind(event.address).f() }",
        );
        let handlers: Vec<&HandlerDecl> = p.handlers.iter().collect();
        let bound = bound_abis(&handlers, &[], &HashMap::new());
        assert!(bound.is_empty());
    }
}
