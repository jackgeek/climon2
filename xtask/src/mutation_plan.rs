//! Mutation selection: which functions a diff adds or changes.
//!
//! A function (or a `const`/`static`, the only mutable items outside a
//! function) is identified by its path and name, for example
//! `<Frame as Display>::fmt`, `Parser::new`, `outer::inner` or `wire::encode`.
//! It is changed when no base item with the same path has the same tokens.
//! Tokens ignore whitespace, ordinary comments and doc comments, so a moved
//! but unchanged function, or a doc-comment-only edit, is not changed. A
//! function's tokens exclude its nested items (including those in const and
//! static initializers), which are compared on their own. An item's lines are
//! the lines its own tokens are on, so a line it shares with a nested item stays
//! and the list is never empty. An item's tokens include each enclosing
//! module's attributes and impl or trait header (generics, bounds, where
//! clause, self type, trait path, attributes; body excluded), so a header edit
//! changes every item inside. Test-only code (`mod tests`, `#[test]` and
//! `#[cfg(test)]` items) is never selected, as the mutator skips it.
//!
//! A changed `struct`, `enum`, `const`, `static` or `type` alias can change how
//! unchanged functions behave, so it also selects every function in the same
//! file that names it: whose own tokens or enclosing headers (an `impl Frame`
//! header names `Frame`) hold its identifier. This is not transitive: a
//! function selected this way selects nothing further (owner decision, #445).
//! Each import of a `use` is compared on its own, and one added, removed or
//! altered selects the same way the functions that name it, by its imported or
//! its local name. A changed glob import (`use x::*`), or one bound to `_`,
//! names nothing that can be found, so it marks the whole file (owner decision,
//! #449).
//!
//! Any other changed item outside a function, `const` or `static` (an item
//! macro such as `macro_rules!`, a trait's method signature or associated type
//! or const without a default, an impl's associated type or macro, a `union`,
//! an `extern` block) has no dependents rule yet, so it marks the whole file to
//! be mutated, as a base that is not Rust does.
//!
//! Each test-only item is compared on its own, inside `mod tests` too. The
//! functions of the file whose names an added, removed or changed test names
//! are reported apart, for the gates that select them (owner decision, #449).

use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use quote::ToTokens;
use std::{
    collections::{BTreeSet, HashMap},
    ops::RangeInclusive,
};
use syn::{
    Attribute, Block, ImplItem, Item, Meta, Stmt, Token, TraitItem, UseTree,
    punctuated::Punctuated,
    visit::{self, Visit},
    visit_mut::{self, VisitMut},
};

/// A function, `const`, `static` or type the diff adds or changes, or a function that
/// names a changed `const`, `static`, type or import, with the lines it owns.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ChangedItem {
    pub name: String,
    pub lines: Vec<RangeInclusive<usize>>,
}

/// What a diff of one file changes.
#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct Changes {
    /// The changed items and the functions that name a changed `const`, `static`, type or
    /// import, in source order.
    pub items: Vec<ChangedItem>,
    /// The base is not Rust, or an item with no dependents rule changed: mutate the whole file.
    pub whole: bool,
    /// `None` when no test-only code was added, removed or changed. Otherwise the functions
    /// of the file that those tests name, in source order, which may be none.
    pub tested: Option<Vec<ChangedItem>>,
}

/// The items of `new` that `old` lacks, the functions of `new` that name a changed `const`,
/// `static`, type or import, whether the whole file changed, and the functions its changed
/// tests name. Errors when `new` is not Rust.
pub(crate) fn changed_items(old: &str, new: &str) -> syn::Result<Changes> {
    let new = records(new)?;
    let Ok(old) = records(old) else {
        return Ok(Changes {
            whole: true,
            ..Changes::default()
        });
    };
    let compared = |record: &&Record| matches!(record.kind, Kind::Other | Kind::Use | Kind::Test);
    let unmatched = unmatched(
        old.iter().filter(compared).collect(),
        new.iter().filter(compared).collect(),
    );
    let names = |kind: Kind| -> BTreeSet<&str> {
        unmatched
            .iter()
            .filter(|record| record.kind == kind)
            .flat_map(|record| record.idents.iter().map(String::as_str))
            .collect()
    };
    let imports = names(Kind::Use);
    let whole = unmatched.iter().any(|record| record.kind == Kind::Other)
        || imports.contains("*")
        || imports.contains("_");
    let tests = unmatched.iter().any(|record| record.kind == Kind::Test);
    let tested = names(Kind::Test);

    let mut base: HashMap<(&str, &str), usize> = HashMap::new();
    for record in old.iter().filter(|record| record.selectable()) {
        *base
            .entry((record.name.as_str(), record.tokens.as_str()))
            .or_default() += 1;
    }
    let new: Vec<_> = new.iter().filter(|record| record.selectable()).collect();
    let changed: Vec<bool> = new
        .iter()
        .map(
            |record| match base.get_mut(&(record.name.as_str(), record.tokens.as_str())) {
                Some(count) if *count > 0 => {
                    *count -= 1;
                    false
                }
                _ => true,
            },
        )
        .collect();
    let mut named = imports;
    named.extend(
        new.iter()
            .zip(&changed)
            .filter(|(record, changed)| {
                **changed && matches!(record.kind, Kind::Value | Kind::Type)
            })
            .map(|(record, _)| record.ident.as_str()),
    );
    let item = |record: &Record| ChangedItem {
        name: record.name.clone(),
        lines: record.lines.clone(),
    };
    Ok(Changes {
        items: new
            .iter()
            .zip(changed)
            .filter(|(record, changed)| *changed || record.names_any(&named))
            .map(|(record, _)| item(record))
            .collect(),
        whole,
        tested: tests.then(|| {
            new.iter()
                .filter(|record| {
                    record.kind == Kind::Function && tested.contains(record.ident.as_str())
                })
                .map(|record| item(record))
                .collect()
        }),
    })
}

/// The records of either side that the other lacks, compared by kind, path and tokens.
fn unmatched<'a>(old: Vec<&'a Record>, new: Vec<&'a Record>) -> Vec<&'a Record> {
    let key = |record: &Record| (record.kind, record.name.clone(), record.tokens.clone());
    let mut lacking = Vec::new();
    for (side, other) in [(&old, &new), (&new, &old)] {
        let mut counts: HashMap<_, usize> = HashMap::new();
        for record in other.iter() {
            *counts.entry(key(record)).or_default() += 1;
        }
        for record in side.iter() {
            match counts.get_mut(&key(record)) {
                Some(count) if *count > 0 => *count -= 1,
                _ => lacking.push(*record),
            }
        }
    }
    lacking
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum Kind {
    Function,
    /// A `const` or `static`.
    Value,
    /// A `struct`, `enum` or `type` alias.
    Type,
    /// One import of a `use`; its idents are the names it binds, `*` for a glob.
    Use,
    /// An item no other record's tokens hold and no dependents rule covers.
    Other,
    /// Test-only code.
    Test,
}

/// A function, `const`, `static` or type in source order, with its own tokens and the lines
/// they are on.
struct Record {
    name: String,
    /// The item's own identifier, which a function names to use it.
    ident: String,
    kind: Kind,
    tokens: String,
    /// Every identifier in its tokens and its enclosing headers.
    idents: BTreeSet<String>,
    lines: Vec<RangeInclusive<usize>>,
}

impl Record {
    /// Whether the record is of an item that may be selected.
    fn selectable(&self) -> bool {
        matches!(self.kind, Kind::Function | Kind::Value | Kind::Type)
    }

    /// Whether this is a function that names one of `items`.
    fn names_any(&self, items: &BTreeSet<&str>) -> bool {
        self.kind == Kind::Function
            && self
                .idents
                .iter()
                .any(|ident| items.contains(ident.as_str()))
    }
}

fn records(source: &str) -> syn::Result<Vec<Record>> {
    let file = syn::parse_file(source)?;
    let mut collector = Collector::default();
    collector.visit_file(&file);
    Ok(collector.records)
}

/// Every line a token of `stream` is on: a group's delimiters and each other token's span.
fn token_lines(stream: TokenStream, lines: &mut BTreeSet<usize>) {
    for token in stream {
        if let TokenTree::Group(group) = &token {
            lines.insert(group.span_open().start().line);
            lines.insert(group.span_close().end().line);
            token_lines(group.stream(), lines);
        } else {
            lines.extend(token.span().start().line..=token.span().end().line);
        }
    }
}

/// Every identifier in `stream`, at any depth.
fn token_idents(stream: TokenStream, idents: &mut BTreeSet<String>) {
    for token in stream {
        match token {
            TokenTree::Group(group) => token_idents(group.stream(), idents),
            TokenTree::Ident(ident) => {
                idents.insert(ident.to_string());
            }
            _ => {}
        }
    }
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
    /// Tokens of each enclosing module's attributes and impl or trait header, body
    /// excluded, which belong to every item's tokens.
    headers: Vec<String>,
    /// The identifiers of each entry of `headers`.
    header_idents: Vec<BTreeSet<String>>,
    /// How many function, `const`, `static` or type bodies enclose the item visited, since the
    /// innermost module, impl or trait, whose tokens their records leave out.
    bodies: usize,
    /// How many test-only modules enclose the item visited.
    tests: usize,
    records: Vec<Record>,
}

impl Collector {
    fn record<T: Clone + ToTokens>(
        &mut self,
        name: &syn::Ident,
        kind: Kind,
        item: &T,
        strip: fn(&mut StripNested, &mut T),
    ) {
        let mut own = item.clone();
        strip(&mut StripNested, &mut own);
        let own = strip_docs(own.to_token_stream());
        let mut lines = BTreeSet::new();
        token_lines(own.clone(), &mut lines);
        let mut idents: BTreeSet<String> = self.header_idents.iter().flatten().cloned().collect();
        token_idents(own.clone(), &mut idents);
        self.records.push(Record {
            name: self.path(&name.to_string()),
            ident: name.to_string(),
            kind,
            idents,
            tokens: self
                .headers
                .iter()
                .cloned()
                .chain([own.to_string()])
                .collect::<Vec<_>>()
                .join("\n"),
            lines: ranges(lines),
        });
    }

    /// Records an item that is never selected itself: its tokens, with the enclosing headers,
    /// and the identifiers in its own tokens.
    fn mark(&mut self, kind: Kind, item: &impl ToTokens) {
        let tokens = strip_docs(item.to_token_stream());
        let mut idents = BTreeSet::new();
        token_idents(tokens.clone(), &mut idents);
        let tokens = tokens.to_string();
        self.records.push(Record {
            name: self.path(""),
            ident: String::new(),
            kind,
            tokens: self
                .headers
                .iter()
                .cloned()
                .chain([tokens])
                .collect::<Vec<_>>()
                .join("\n"),
            idents,
            lines: Vec::new(),
        });
    }

    /// Records each import of `item` on its own, with its attributes and visibility, and the
    /// names it binds: its imported and local names, or `*` for a glob.
    fn uses(&mut self, item: &syn::ItemUse) {
        let mut prefix: Vec<String> = item
            .attrs
            .iter()
            .map(|attr| strip_docs(attr.to_token_stream()).to_string())
            .collect();
        prefix.push(item.vis.to_token_stream().to_string());
        let leading = if item.leading_colon.is_some() {
            "::"
        } else {
            ""
        };
        let mut imports = Vec::new();
        flatten(&item.tree, leading.to_owned(), None, &mut imports);
        for (path, idents) in imports {
            self.records.push(Record {
                name: self.path(""),
                ident: String::new(),
                kind: Kind::Use,
                tokens: format!("{} {path}", prefix.join(" ")),
                idents,
                lines: Vec::new(),
            });
        }
    }

    /// Records a test-only item: each item of a test-only module on its own, and the module
    /// itself without them.
    fn test_item(&mut self, item: &Item) {
        let Item::Mod(module) = item else {
            return self.mark(Kind::Test, item);
        };
        let mut header = module.clone();
        let items = header.content.take().map(|(_, items)| items);
        self.mark(Kind::Test, &header);
        self.tests += 1;
        self.within(module.ident.to_string(), |c| {
            for item in items.iter().flatten() {
                c.visit_item(item);
            }
        });
        self.tests -= 1;
    }

    /// Visits the body of a recorded item, whose record holds the tokens of the items in it
    /// that are not compared on their own.
    fn inside(&mut self, name: String, visit: impl FnOnce(&mut Self)) {
        self.bodies += 1;
        self.within(name, visit);
        self.bodies -= 1;
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

    /// Visits a module, impl or trait whose `header` belongs to every item inside it.
    fn enclosed(&mut self, name: String, header: TokenStream, visit: impl FnOnce(&mut Self)) {
        let header = strip_docs(header);
        let mut idents = BTreeSet::new();
        token_idents(header.clone(), &mut idents);
        self.headers.push(header.to_string());
        self.header_idents.push(idents);
        let bodies = std::mem::take(&mut self.bodies);
        self.within(name, visit);
        self.bodies = bodies;
        self.headers.pop();
        self.header_idents.pop();
    }
}

impl<'ast> Visit<'ast> for Collector {
    fn visit_item(&mut self, item: &'ast Item) {
        if self.tests > 0
            || test_only(item_attrs(item))
            || matches!(item, Item::Mod(module) if module.ident == "tests")
        {
            return self.test_item(item);
        }
        // Inside a body, such an item is in the enclosing record's own tokens.
        if self.bodies == 0 {
            match item {
                Item::Use(item) => self.uses(item),
                Item::Macro(_)
                | Item::Union(_)
                | Item::ExternCrate(_)
                | Item::ForeignMod(_)
                | Item::TraitAlias(_)
                | Item::Verbatim(_) => self.mark(Kind::Other, item),
                _ => {}
            }
        }
        visit::visit_item(self, item);
    }

    fn visit_impl_item(&mut self, item: &'ast ImplItem) {
        let attrs: &[Attribute] = match item {
            ImplItem::Const(item) => &item.attrs,
            ImplItem::Fn(item) => &item.attrs,
            ImplItem::Type(item) => &item.attrs,
            ImplItem::Macro(item) => &item.attrs,
            _ => &[],
        };
        if test_only(attrs) {
            return self.mark(Kind::Test, item);
        }
        if !matches!(item, ImplItem::Const(_) | ImplItem::Fn(_)) {
            self.mark(Kind::Other, item);
        }
        visit::visit_impl_item(self, item);
    }

    fn visit_trait_item(&mut self, item: &'ast TraitItem) {
        let (attrs, recorded): (&[Attribute], bool) = match item {
            TraitItem::Const(item) => (&item.attrs, item.default.is_some()),
            TraitItem::Fn(item) => (&item.attrs, item.default.is_some()),
            TraitItem::Type(item) => (&item.attrs, false),
            TraitItem::Macro(item) => (&item.attrs, false),
            _ => (&[], false),
        };
        if test_only(attrs) {
            return self.mark(Kind::Test, item);
        }
        if !recorded {
            self.mark(Kind::Other, item);
        }
        visit::visit_trait_item(self, item);
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if !test_only(&item.attrs) && item.ident != "tests" {
            let attributes = item.attrs.iter().map(ToTokens::to_token_stream).collect();
            self.enclosed(item.ident.to_string(), attributes, |c| {
                visit::visit_item_mod(c, item)
            });
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
        self.enclosed(name, header.to_token_stream(), |c| {
            visit::visit_item_impl(c, item)
        });
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        if !test_only(&item.attrs) {
            let mut header = item.clone();
            header.items.clear();
            self.enclosed(item.ident.to_string(), header.to_token_stream(), |c| {
                visit::visit_item_trait(c, item)
            });
        }
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if !test_only(&item.attrs) {
            self.record(
                &item.sig.ident,
                Kind::Function,
                item,
                StripNested::visit_item_fn_mut,
            );
            self.inside(item.sig.ident.to_string(), |c| {
                visit::visit_item_fn(c, item)
            });
        }
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if !test_only(&item.attrs) {
            self.record(
                &item.sig.ident,
                Kind::Function,
                item,
                StripNested::visit_impl_item_fn_mut,
            );
            self.inside(item.sig.ident.to_string(), |c| {
                visit::visit_impl_item_fn(c, item)
            });
        }
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        if item.default.is_some() && !test_only(&item.attrs) {
            self.record(
                &item.sig.ident,
                Kind::Function,
                item,
                StripNested::visit_trait_item_fn_mut,
            );
            self.inside(item.sig.ident.to_string(), |c| {
                visit::visit_trait_item_fn(c, item)
            });
        }
    }

    fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
        if !test_only(&item.attrs) {
            self.record(
                &item.ident,
                Kind::Value,
                item,
                StripNested::visit_item_const_mut,
            );
            self.inside(item.ident.to_string(), |c| visit::visit_item_const(c, item));
        }
    }

    fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
        if !test_only(&item.attrs) {
            self.record(
                &item.ident,
                Kind::Value,
                item,
                StripNested::visit_item_static_mut,
            );
            self.inside(item.ident.to_string(), |c| {
                visit::visit_item_static(c, item)
            });
        }
    }

    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        if !test_only(&item.attrs) {
            self.record(
                &item.ident,
                Kind::Type,
                item,
                StripNested::visit_item_struct_mut,
            );
            self.inside(item.ident.to_string(), |c| {
                visit::visit_item_struct(c, item)
            });
        }
    }

    fn visit_item_enum(&mut self, item: &'ast syn::ItemEnum) {
        if !test_only(&item.attrs) {
            self.record(
                &item.ident,
                Kind::Type,
                item,
                StripNested::visit_item_enum_mut,
            );
            self.inside(item.ident.to_string(), |c| visit::visit_item_enum(c, item));
        }
    }

    fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
        if !test_only(&item.attrs) {
            self.record(
                &item.ident,
                Kind::Type,
                item,
                StripNested::visit_item_type_mut,
            );
            self.inside(item.ident.to_string(), |c| visit::visit_item_type(c, item));
        }
    }

    fn visit_impl_item_const(&mut self, item: &'ast syn::ImplItemConst) {
        if !test_only(&item.attrs) {
            self.record(
                &item.ident,
                Kind::Value,
                item,
                StripNested::visit_impl_item_const_mut,
            );
            self.inside(item.ident.to_string(), |c| {
                visit::visit_impl_item_const(c, item)
            });
        }
    }

    fn visit_trait_item_const(&mut self, item: &'ast syn::TraitItemConst) {
        if item.default.is_some() && !test_only(&item.attrs) {
            self.record(
                &item.ident,
                Kind::Value,
                item,
                StripNested::visit_trait_item_const_mut,
            );
            self.inside(item.ident.to_string(), |c| {
                visit::visit_trait_item_const(c, item)
            });
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

/// Each import of `tree` under `path`: its path, and the names it binds.
fn flatten(
    tree: &UseTree,
    path: String,
    parent: Option<&syn::Ident>,
    imports: &mut Vec<(String, BTreeSet<String>)>,
) {
    // `self` in a group binds its parent's name.
    let name = |ident: &syn::Ident| match parent {
        Some(parent) if ident == "self" => parent.to_string(),
        _ => ident.to_string(),
    };
    match tree {
        UseTree::Path(tree) => flatten(
            &tree.tree,
            format!("{path}{}::", tree.ident),
            Some(&tree.ident),
            imports,
        ),
        UseTree::Name(tree) => imports.push((
            format!("{path}{}", tree.ident),
            BTreeSet::from([name(&tree.ident)]),
        )),
        UseTree::Rename(tree) => imports.push((
            format!("{path}{} as {}", tree.ident, tree.rename),
            BTreeSet::from([name(&tree.ident), tree.rename.to_string()]),
        )),
        UseTree::Glob(_) => imports.push((format!("{path}*"), BTreeSet::from(["*".to_owned()]))),
        UseTree::Group(group) => {
            for tree in &group.items {
                flatten(tree, path.clone(), parent, imports);
            }
        }
    }
}

fn item_attrs(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        Item::ExternCrate(item) => &item.attrs,
        Item::Fn(item) => &item.attrs,
        Item::ForeignMod(item) => &item.attrs,
        Item::Impl(item) => &item.attrs,
        Item::Macro(item) => &item.attrs,
        Item::Mod(item) => &item.attrs,
        Item::Static(item) => &item.attrs,
        Item::Struct(item) => &item.attrs,
        Item::Trait(item) => &item.attrs,
        Item::TraitAlias(item) => &item.attrs,
        Item::Type(item) => &item.attrs,
        Item::Union(item) => &item.attrs,
        Item::Use(item) => &item.attrs,
        _ => &[],
    }
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

    /// The selected items, where the change does not mark the whole file.
    fn selected(old: &str, new: &str) -> Vec<ChangedItem> {
        let changes = changed_items(old, new).unwrap();
        assert!(!changes.whole, "{changes:?}");
        changes.items
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
        assert_eq!(selected(BASE, &new), vec![item("added", &[9..=11])]);
    }

    #[test]
    fn an_edited_body_is_changed() {
        let new = BASE.replace("    2", "    3");
        assert_eq!(selected(BASE, &new), vec![item("edited", &[5..=7])]);
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
        assert_eq!(selected(BASE, new), vec![]);
    }

    /// Owner choice: doc comments carry no mutation sites, so editing only them changes nothing.
    #[test]
    fn a_doc_comment_only_edit_is_not_changed() {
        let old = format!("/// Old words.\n{BASE}");
        let new = format!("/// New words,\n/// over two lines.\n{BASE}");
        assert_eq!(selected(&old, &new), vec![]);
    }

    #[test]
    fn a_deleted_function_leaves_nothing_to_mutate() {
        let new = "fn kept() -> u8 {\n    1\n}\n";
        assert_eq!(selected(BASE, new), vec![]);
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
            selected(old, &new),
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
            selected(old, &new),
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
            selected(old, &bound),
            vec![
                item("W<T>::first", &[3..=3]),
                item("W<T>::second", &[4..=4])
            ]
        );
        let method = old.replace("{ 2 }", "{ 3 }");
        assert_eq!(selected(old, &method), vec![item("W<T>::second", &[4..=4])]);
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
            selected(old, &inner_only),
            vec![item("outer::inner", &[2..=4])]
        );
        let outer_only = old.replace("inner() + 1", "inner() + 2");
        assert_eq!(
            selected(old, &outer_only),
            vec![item("outer", &[1..=1, 5..=6])]
        );
    }

    /// Review 1: a function in a const or static initializer is an item too.
    #[test]
    fn a_function_inside_a_const_or_static_initializer_is_compared() {
        let old = "\
static H: fn() -> u8 = {
    fn h() -> u8 { 1 }
    h
};
";
        let new = old.replace("{ 1 }", "{ 2 }");
        assert_eq!(selected(old, &new), vec![item("H::h", &[2..=2])]);
    }

    /// Review 2 and 3: a parent keeps every line its own tokens are on, even lines
    /// shared with nested items, so its list is never empty.
    #[test]
    fn a_parent_keeps_lines_it_shares_with_nested_items() {
        let old = "\
fn outer() -> usize {
    let v = { const N: usize = 4; [0u8; N].len() + 1 };
    v
}
";
        let new = old.replace("+ 1", "+ 2");
        assert_eq!(selected(old, &new), vec![item("outer", &[1..=4])]);
        let old = "\
fn outer() -> u8 { fn a() -> u8 { 1 }
    fn b() -> u8 { 2 }
    a() + b() + { fn c() -> u8 { 3 } c() } }
";
        let new = old.replace("a() + b()", "a() - b()");
        assert_eq!(selected(old, &new), vec![item("outer", &[1..=1, 3..=3])]);
    }

    /// Review 4: a trait header edit changes every default method, as an impl header does.
    #[test]
    fn an_edited_trait_header_changes_its_default_methods() {
        let old = "\
trait T: Clone {
    fn d(&self) -> u8 { 1 }
}
";
        let new = old.replace("Clone", "Copy");
        assert_eq!(selected(old, &new), vec![item("T::d", &[2..=2])]);
    }

    /// Review 5: an enclosing module's or impl's attributes belong to its items; doc comments do not.
    #[test]
    fn enclosing_attributes_but_not_doc_comments_belong_to_items() {
        let old = "\
/// Unix only.
#[cfg(unix)]
mod m {
    pub fn g() -> u8 { 1 }
}
";
        let platform = old.replace("cfg(unix)", "cfg(windows)");
        assert_eq!(selected(old, &platform), vec![item("m::g", &[4..=4])]);
        let docs = old.replace("Unix only.", "Only on Unix.");
        assert_eq!(selected(old, &docs), vec![]);
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
        assert_eq!(selected(old, new), vec![item("production_only", &[9..=10])]);
    }

    /// #445 owner decision: a changed type, const or static selects every function in the
    /// file that names it, enclosing impl headers included; the selection is not transitive.
    #[test]
    fn a_changed_type_or_const_selects_the_functions_that_name_it() {
        let old = "\
struct Frame { width: u8 }
const LIMIT: u8 = 1;
impl Frame {
    fn wide(&self) -> bool { self.width > 1 }
}
fn limited(value: u8) -> bool { value < LIMIT }
fn caller() -> bool { limited(2) }
fn unrelated() -> u8 { 3 }
";
        let field = old.replace("width: u8", "width: u16");
        assert_eq!(
            selected(old, &field),
            vec![item("Frame", &[1..=1]), item("Frame::wide", &[4..=4])]
        );
        let constant = old.replace("LIMIT: u8 = 1", "LIMIT: u8 = 2");
        assert_eq!(
            selected(old, &constant),
            vec![item("LIMIT", &[2..=2]), item("limited", &[6..=6])]
        );
        let old = "\
enum Mode { A }
type Size = u8;
fn mode() -> Mode { Mode::A }
fn size() -> Size { 1 }
";
        let new = old.replace("{ A }", "{ A, B }").replace("= u8", "= u16");
        assert_eq!(
            selected(old, &new),
            vec![
                item("Mode", &[1..=1]),
                item("Size", &[2..=2]),
                item("mode", &[3..=3]),
                item("size", &[4..=4]),
            ]
        );
    }

    /// #445: a base that is not Rust marks the whole file.
    #[test]
    fn an_unparsable_base_marks_the_whole_file() {
        let changes = changed_items("fn broken(", BASE).unwrap();
        assert!(changes.whole, "{changes:?}");
    }

    /// #449 review: a changed item with no dependents rule marks the whole file.
    #[test]
    fn a_changed_item_of_an_unselected_kind_marks_the_whole_file() {
        let old = "\
macro_rules! twice { ($x:expr) => { $x * 2 } }
trait T {
    type Out;
    const N: u8;
    fn required(&self) -> u8;
}
struct S;
impl T for S {
    type Out = u8;
    const N: u8 = 1;
    fn required(&self) -> u8 { 1 }
}
union U { a: u8 }
extern \"C\" { fn abs(x: i32) -> i32; }
fn f() -> u8 { 1 }
";
        for (from, to) in [
            ("$x * 2", "$x * 3"),
            ("fn required(&self) -> u8;", "fn required(&self) -> u16;"),
            ("    type Out;", "    type Out: Copy;"),
            ("    const N: u8;", "    const N: u16;"),
            ("type Out = u8;", "type Out = u16;"),
            ("a: u8 }", "a: u16 }"),
            ("x: i32) -> i32", "x: i64) -> i64"),
            ("fn f() -> u8 { 1 }", "use std::io::*;\nfn f() -> u8 { 1 }"),
            (
                "fn f() -> u8 { 1 }",
                "use std::io::Read as _;\nfn f() -> u8 { 1 }",
            ),
        ] {
            let new = old.replace(from, to);
            assert_ne!(new, old, "{from}");
            let changes = changed_items(old, &new).unwrap();
            assert!(changes.whole, "{from}: {changes:?}");
        }
        assert!(!changed_items(old, old).unwrap().whole);
        // Removing one is a change too.
        let removed = old.replace("union U { a: u8 }\n", "");
        assert!(changed_items(old, &removed).unwrap().whole);
    }

    /// #449 owner decision: an import added, removed or altered selects the functions that
    /// name it by its imported or its local name, impl headers included, one step only.
    #[test]
    fn a_changed_import_selects_the_functions_that_name_it() {
        let old = "\
use std::fmt::{self, Display};
use std::io::Read as Input;
fn show() -> u8 { let _: Option<&dyn Display> = None; 1 }
fn read(_: &dyn Input) {}
fn format() -> fmt::Result { Ok(()) }
struct W;
impl Display for W {
    fn fmt(&self, _: &mut fmt::Formatter) -> fmt::Result { Ok(()) }
}
fn other() {}
";
        let cases: [(&str, &str, &[ChangedItem]); 5] = [
            // An added import that nothing names selects nothing.
            ("{self, Display}", "{self, Display, Write}", &[]),
            (
                "use std::fmt::{self, Display};",
                "use core::fmt::{self, Display};",
                &[
                    item("show", &[3..=3]),
                    item("format", &[5..=5]),
                    item("<W as Display>::fmt", &[8..=8]),
                ],
            ),
            ("Read as Input", "Read as Source", &[item("read", &[4..=4])]),
            (
                "use std::io::Read as Input;\n",
                "",
                &[item("read", &[3..=3])],
            ),
            (
                "{self, Display}",
                "{self}",
                &[
                    item("show", &[3..=3]),
                    item("<W as Display>::fmt", &[8..=8]),
                ],
            ),
        ];
        for (from, to, expected) in cases {
            let new = old.replace(from, to);
            assert_eq!(selected(old, &new), expected, "{from} -> {to}");
        }
    }

    /// #449 review: an item inside a body is in its function's own tokens, so it changes only
    /// that function.
    #[test]
    fn an_unselected_kind_inside_a_function_changes_only_that_function() {
        let old = "fn f() -> u8 {\n    use std::cmp::max;\n    max(1, 2)\n}\nfn g() {}\n";
        let new = old.replace("cmp::max", "cmp::{max, min}");
        assert_eq!(selected(old, &new), vec![item("f", &[1..=4])]);
    }

    /// #449 owner decision: a test added, removed or changed reports the functions of the file
    /// its body names, methods included, one step only; it selects no item itself.
    #[test]
    fn a_changed_test_reports_the_functions_it_names() {
        let old = format!(
            "{BASE}\n#[cfg(test)]\nmod tests {{\n    #[test]\n    fn proves() {{ assert_eq!(super::kept(), 1); }}\n    #[test]\n    fn checks() {{ assert_eq!(1, 1); }}\n}}\n"
        );
        assert_eq!(changed_items(&old, &old).unwrap(), Changes::default());
        let kept = vec![item("kept", &[1..=3])];
        for (new, tested) in [
            (
                old.replace("super::kept(), 1", "super::kept(), 2"),
                kept.clone(),
            ),
            (
                old.replace("fn proves() { assert_eq!(super::kept(), 1); }", ""),
                kept,
            ),
            (
                old.replace("assert_eq!(1, 1)", "assert_eq!(super::edited(), 2)"),
                vec![item("edited", &[5..=7])],
            ),
            (
                old.replace("assert_eq!(1, 1)", "assert_eq!(2, 2)"),
                Vec::new(),
            ),
            (
                format!("{old}#[test]\nfn more() {{ edited(); }}\n"),
                vec![item("edited", &[5..=7])],
            ),
        ] {
            assert_eq!(
                changed_items(&old, &new).unwrap(),
                Changes {
                    items: Vec::new(),
                    whole: false,
                    tested: Some(tested),
                },
                "{new}"
            );
        }
    }

    #[test]
    fn source_that_is_not_rust_is_an_error() {
        assert!(changed_items("", "fn broken(").is_err());
    }
}
