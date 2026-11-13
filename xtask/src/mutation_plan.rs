//! Ticket-gate mutation selection: which functions a diff adds or changes.
//!
//! A function (or a `const`/`static`, the only mutable items outside a
//! function) is identified by its path and name, for example
//! `<Frame as Display>::fmt`, `Parser::new`, `outer::inner` or `wire::encode`.
//! It is changed when no base item with the same path has the same tokens.
//! Tokens ignore whitespace, ordinary comments and doc comments, so a moved
//! but unchanged function, or a doc-comment-only edit, is not changed. A
//! function's tokens exclude its nested items, which are compared on their
//! own; its lines exclude theirs. An item's tokens include each enclosing impl
//! header (generics, bounds, where clause, self type, trait path; body
//! excluded), so a header edit changes every item in that impl. Test-only code (`mod tests`, `#[test]` and
//! `#[cfg(test)]` items) is skipped, as the mutator skips it.

use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use quote::ToTokens;
use std::{
    collections::{BTreeSet, HashMap},
    ops::RangeInclusive,
};
use syn::{
    Attribute, Block, Item, Meta, Stmt, Token,
    punctuated::Punctuated,
    visit::{self, Visit},
    visit_mut::{self, VisitMut},
};

/// A function, `const` or `static` the diff adds or changes, with the lines it owns.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ChangedItem {
    pub name: String,
    pub lines: Vec<RangeInclusive<usize>>,
}

/// The items of `new` that `old` lacks. Errors when `new` is not Rust.
/// Unparsable base source has no items, so every item of `new` is changed.
pub(crate) fn changed_items(old: &str, new: &str) -> syn::Result<Vec<ChangedItem>> {
    let new = records(new)?;
    let mut base: HashMap<(String, String), usize> = HashMap::new();
    for record in records(old).unwrap_or_default() {
        *base.entry((record.name, record.tokens)).or_default() += 1;
    }
    let mut changed = Vec::new();
    for record in &new {
        match base.get_mut(&(record.name.clone(), record.tokens.clone())) {
            Some(count) if *count > 0 => *count -= 1,
            _ => changed.push(ChangedItem {
                name: record.name.clone(),
                lines: own_lines(record, &new),
            }),
        }
    }
    Ok(changed)
}

/// A function, `const` or `static` in source order, with its own tokens and whole span.
struct Record {
    name: String,
    tokens: String,
    lines: RangeInclusive<usize>,
}

fn records(source: &str) -> syn::Result<Vec<Record>> {
    let file = syn::parse_file(source)?;
    let mut collector = Collector::default();
    collector.visit_file(&file);
    Ok(collector.records)
}

/// The record's lines, less those of every item nested inside it.
fn own_lines(record: &Record, all: &[Record]) -> Vec<RangeInclusive<usize>> {
    let mut lines: BTreeSet<usize> = record.lines.clone().collect();
    for nested in all.iter().filter(|other| {
        other.lines != record.lines
            && record.lines.contains(other.lines.start())
            && record.lines.contains(other.lines.end())
    }) {
        for line in nested.lines.clone() {
            lines.remove(&line);
        }
    }
    ranges(lines)
}

fn ranges(lines: BTreeSet<usize>) -> Vec<RangeInclusive<usize>> {
    let mut ranges: Vec<RangeInclusive<usize>> = Vec::new();
    for line in lines {
        match ranges.last_mut() {
            Some(last) if *last.end() + 1 == line => *last = *last.start()..=line,
            _ => ranges.push(line..=line),
        }
    }
    ranges
}

#[derive(Default)]
struct Collector {
    scope: Vec<String>,
    /// Tokens of each enclosing impl header, body excluded, which belong to every method's tokens.
    headers: Vec<String>,
    records: Vec<Record>,
}

impl Collector {
    fn record<T: Clone + ToTokens>(
        &mut self,
        name: &syn::Ident,
        item: &T,
        strip: fn(&mut StripNested, &mut T),
    ) {
        let mut own = item.clone();
        strip(&mut StripNested, &mut own);
        let whole = strip_docs(item.to_token_stream());
        let mut tokens = whole.clone().into_iter();
        let start = tokens.next().map_or(1, |first| first.span().start().line);
        let end = tokens.last().map_or(start, |last| last.span().end().line);
        self.records.push(Record {
            name: self.path(&name.to_string()),
            tokens: self
                .headers
                .iter()
                .cloned()
                .chain([strip_docs(own.to_token_stream()).to_string()])
                .collect::<Vec<_>>()
                .join("\n"),
            lines: start..=end,
        });
    }

    fn path(&self, name: &str) -> String {
        self.scope
            .iter()
            .map(String::as_str)
            .chain([name])
            .collect::<Vec<_>>()
            .join("::")
    }

    fn within(&mut self, name: String, visit: impl FnOnce(&mut Self)) {
        self.scope.push(name);
        visit(self);
        self.scope.pop();
    }
}

impl<'ast> Visit<'ast> for Collector {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if !test_only(&item.attrs) && item.ident != "tests" {
            self.within(item.ident.to_string(), |c| visit::visit_item_mod(c, item));
        }
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if test_only(&item.attrs) {
            return;
        }
        let ty = compact(&item.self_ty);
        let name = match &item.trait_ {
            Some((_, path, _)) => format!("<{ty} as {}>", compact(path)),
            None => ty,
        };
        let mut header = item.clone();
        header.items.clear();
        self.headers
            .push(strip_docs(header.to_token_stream()).to_string());
        self.within(name, |c| visit::visit_item_impl(c, item));
        self.headers.pop();
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        if !test_only(&item.attrs) {
            self.within(item.ident.to_string(), |c| visit::visit_item_trait(c, item));
        }
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if !test_only(&item.attrs) {
            self.record(&item.sig.ident, item, StripNested::visit_item_fn_mut);
            self.within(item.sig.ident.to_string(), |c| {
                visit::visit_item_fn(c, item)
            });
        }
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if !test_only(&item.attrs) {
            self.record(&item.sig.ident, item, StripNested::visit_impl_item_fn_mut);
            self.within(item.sig.ident.to_string(), |c| {
                visit::visit_impl_item_fn(c, item)
            });
        }
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        if item.default.is_some() && !test_only(&item.attrs) {
            self.record(&item.sig.ident, item, StripNested::visit_trait_item_fn_mut);
            self.within(item.sig.ident.to_string(), |c| {
                visit::visit_trait_item_fn(c, item)
            });
        }
    }

    fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
        if !test_only(&item.attrs) {
            self.record(&item.ident, item, StripNested::visit_item_const_mut);
        }
    }

    fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
        if !test_only(&item.attrs) {
            self.record(&item.ident, item, StripNested::visit_item_static_mut);
        }
    }

    fn visit_impl_item_const(&mut self, item: &'ast syn::ImplItemConst) {
        if !test_only(&item.attrs) {
            self.record(&item.ident, item, StripNested::visit_impl_item_const_mut);
        }
    }

    fn visit_trait_item_const(&mut self, item: &'ast syn::TraitItemConst) {
        if item.default.is_some() && !test_only(&item.attrs) {
            self.record(&item.ident, item, StripNested::visit_trait_item_const_mut);
        }
    }
}

/// Removes the nested items that are compared on their own.
struct StripNested;

impl VisitMut for StripNested {
    fn visit_block_mut(&mut self, block: &mut Block) {
        block.stmts.retain(|stmt| {
            !matches!(
                stmt,
                Stmt::Item(
                    Item::Fn(_)
                        | Item::Const(_)
                        | Item::Static(_)
                        | Item::Impl(_)
                        | Item::Trait(_)
                        | Item::Mod(_)
                )
            )
        });
        visit_mut::visit_block_mut(self, block);
    }
}

/// Tokens as written, keeping a space only between two words: `Session<'_, C>` not `Session < '_ , C >`.
fn compact(tokens: &impl ToTokens) -> String {
    let text = tokens.to_token_stream().to_string();
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let mut compacted = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != ' ' || (word(compacted.chars().last()) && word(chars.peek().copied())) {
            compacted.push(c);
        }
    }
    compacted
}

/// `#[test]`, `#[cfg(test)]` or `#[cfg(all(.., test, ..))]`.
fn test_only(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("test")
            || (attr.path().is_ident("cfg")
                && attr
                    .parse_args::<Meta>()
                    .is_ok_and(|meta| requires_test(&meta)))
    })
}

fn requires_test(meta: &Meta) -> bool {
    match meta {
        Meta::Path(path) => path.is_ident("test"),
        Meta::List(list) if list.path.is_ident("all") => list
            .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            .is_ok_and(|nested| nested.iter().any(requires_test)),
        _ => false,
    }
}

/// The stream without `#[doc ..]` or `#![doc ..]` attributes, at any depth.
fn strip_docs(stream: TokenStream) -> TokenStream {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut kept = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if let Some(length) = doc_attribute_length(&tokens[index..]) {
            index += length;
            continue;
        }
        kept.push(match &tokens[index] {
            TokenTree::Group(group) => {
                let mut stripped = Group::new(group.delimiter(), strip_docs(group.stream()));
                stripped.set_span(group.span());
                TokenTree::Group(stripped)
            }
            token => token.clone(),
        });
        index += 1;
    }
    kept.into_iter().collect()
}

fn doc_attribute_length(tokens: &[TokenTree]) -> Option<usize> {
    let is_punct = |token: Option<&TokenTree>, ch| matches!(token, Some(TokenTree::Punct(punct)) if punct.as_char() == ch);
    if !is_punct(tokens.first(), '#') {
        return None;
    }
    let bracket = 1 + usize::from(is_punct(tokens.get(1), '!'));
    let Some(TokenTree::Group(group)) = tokens.get(bracket) else {
        return None;
    };
    let first = group.stream().into_iter().next();
    (group.delimiter() == Delimiter::Bracket
        && matches!(first, Some(TokenTree::Ident(ident)) if ident == "doc"))
    .then_some(bracket + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, lines: &[RangeInclusive<usize>]) -> ChangedItem {
        ChangedItem {
            name: name.to_owned(),
            lines: lines.to_vec(),
        }
    }

    const BASE: &str = "\
fn kept() -> u8 {
    1
}

fn edited() -> u8 {
    2
}
";

    #[test]
    fn a_new_function_is_changed_and_an_untouched_one_is_not() {
        let new = format!("{BASE}\nfn added() -> bool {{\n    true\n}}\n");
        assert_eq!(
            changed_items(BASE, &new).unwrap(),
            vec![item("added", &[9..=11])]
        );
    }

    #[test]
    fn an_edited_body_is_changed() {
        let new = BASE.replace("    2", "    3");
        assert_eq!(
            changed_items(BASE, &new).unwrap(),
            vec![item("edited", &[5..=7])]
        );
    }

    #[test]
    fn a_function_moved_unchanged_within_the_file_is_not_changed() {
        let new = "\
fn edited() -> u8 {
    2
}

// Reordered, with a new comment and different layout.
fn kept()  ->  u8 { 1 }
";
        assert_eq!(changed_items(BASE, new).unwrap(), vec![]);
    }

    /// Owner choice: doc comments carry no mutation sites, so editing only them changes nothing.
    #[test]
    fn a_doc_comment_only_edit_is_not_changed() {
        let old = format!("/// Old words.\n{BASE}");
        let new = format!("/// New words,\n/// over two lines.\n{BASE}");
        assert_eq!(changed_items(&old, &new).unwrap(), vec![]);
    }

    #[test]
    fn a_deleted_function_leaves_nothing_to_mutate() {
        let new = "fn kept() -> u8 {\n    1\n}\n";
        assert_eq!(changed_items(BASE, new).unwrap(), vec![]);
    }

    #[test]
    fn an_edited_impl_method_is_named_by_its_impl_and_does_not_collide() {
        let old = "\
struct A;
struct B<T>(T);
impl A {
    fn new() -> u8 { 1 }
}
impl<'a> B<&'a u8> {
    fn new() -> u8 { 1 }
}
impl std::fmt::Display for A {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str(\"a\") }
}
";
        let new = old
            .replace(
                "u8> {\n    fn new() -> u8 { 1 }",
                "u8> {\n    fn new() -> u8 { 2 }",
            )
            .replace("\"a\"", "\"A\"");
        assert_eq!(
            changed_items(old, &new).unwrap(),
            vec![
                item("B<&'a u8>::new", &[7..=7]),
                item("<A as std::fmt::Display>::fmt", &[10..=10]),
            ]
        );
    }

    #[test]
    fn trait_defaults_module_functions_consts_and_statics_are_items() {
        let old = "\
trait T {
    fn d(&self) -> u8 { 1 }
}
mod m {
    pub fn g() -> u8 { 1 }
    pub const LIMIT: u8 = 1;
}
static FLAG: bool = true;
";
        let new = old.replace("1", "2").replace("true", "false");
        assert_eq!(
            changed_items(old, &new).unwrap(),
            vec![
                item("T::d", &[2..=2]),
                item("m::g", &[5..=5]),
                item("m::LIMIT", &[6..=6]),
                item("FLAG", &[8..=8]),
            ]
        );
    }

    /// Owner decision: an impl header edit changes every method in that impl.
    #[test]
    fn an_edited_impl_header_changes_all_its_methods_and_an_edited_method_only_itself() {
        let old = "\
struct W<T>(T);
impl<T: Clone> W<T> {
    fn first(&self) -> u8 { 1 }
    fn second(&self) -> u8 { 2 }
}
";
        let bound = old.replace("T: Clone", "T: Copy");
        assert_eq!(
            changed_items(old, &bound).unwrap(),
            vec![
                item("W<T>::first", &[3..=3]),
                item("W<T>::second", &[4..=4])
            ]
        );
        let method = old.replace("{ 2 }", "{ 3 }");
        assert_eq!(
            changed_items(old, &method).unwrap(),
            vec![item("W<T>::second", &[4..=4])]
        );
    }

    #[test]
    fn a_nested_function_is_compared_and_scoped_apart_from_its_parent() {
        let old = "\
fn outer() -> u8 {
    fn inner() -> u8 {
        1
    }
    inner() + 1
}
";
        let inner_only = old.replace("        1", "        2");
        assert_eq!(
            changed_items(old, &inner_only).unwrap(),
            vec![item("outer::inner", &[2..=4])]
        );
        let outer_only = old.replace("inner() + 1", "inner() + 2");
        assert_eq!(
            changed_items(old, &outer_only).unwrap(),
            vec![item("outer", &[1..=1, 5..=6])]
        );
    }

    #[test]
    fn test_only_code_is_never_changed() {
        let old = "fn shipped() {}\n";
        let new = "\
fn shipped() {}
#[cfg(test)]
mod checks { fn helper() -> u8 { 1 } }
mod tests { fn helper() -> u8 { 1 } }
#[test]
fn proves() {}
#[cfg(all(test, unix))]
fn unix_helper() {}
#[cfg(not(test))]
fn production_only() {}
";
        assert_eq!(
            changed_items(old, new).unwrap(),
            vec![item("production_only", &[9..=10])]
        );
    }

    #[test]
    fn source_that_is_not_rust_is_an_error() {
        assert!(changed_items("", "fn broken(").is_err());
    }
}
