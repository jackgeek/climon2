//! ADR 0007: `climon2-vt` exposes only climon2-owned types. Its public
//! interface (public functions, fields, variants, aliases, constants and
//! re-exports, and the trait impls of its public types) never names a type
//! of the engine, `alacritty_terminal`: not by path, not by a name a `use`
//! binds to one, and not through a crate type alias of one.

use std::{collections::BTreeSet, fs, path::Path};
use syn::{
    File, Ident, ImplItem, Item, ItemImpl, Type, UseTree, Visibility,
    visit::{self, Visit},
};

const ENGINE: &str = "alacritty_terminal";

/// The names a file's `use` items bind to engine items; `*` for a glob.
fn engine_imports(file: &File) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for item in &file.items {
        if let Item::Use(item) = item {
            bind(&item.tree, false, None, &mut names);
        }
    }
    names
}

fn bind(tree: &UseTree, engine: bool, parent: Option<&Ident>, names: &mut BTreeSet<String>) {
    match tree {
        UseTree::Path(path) => bind(
            &path.tree,
            engine || path.ident == ENGINE,
            Some(&path.ident),
            names,
        ),
        UseTree::Name(name) if engine => {
            let bound = match parent {
                Some(parent) if name.ident == "self" => parent,
                _ => &name.ident,
            };
            names.insert(bound.to_string());
        }
        UseTree::Rename(rename) if engine => {
            names.insert(rename.rename.to_string());
        }
        UseTree::Glob(_) if engine => {
            names.insert("*".to_owned());
        }
        UseTree::Group(group) => group
            .items
            .iter()
            .for_each(|item| bind(item, engine, parent, names)),
        _ => {}
    }
}

/// Every identifier a `use` tree names.
fn use_idents(tree: &UseTree, idents: &mut Vec<String>) {
    match tree {
        UseTree::Path(path) => {
            idents.push(path.ident.to_string());
            use_idents(&path.tree, idents);
        }
        UseTree::Name(name) => idents.push(name.ident.to_string()),
        UseTree::Rename(rename) => idents.push(rename.ident.to_string()),
        UseTree::Glob(_) => {}
        UseTree::Group(group) => group.items.iter().for_each(|item| use_idents(item, idents)),
    }
}

/// The paths in what it visits that start with an engine name.
struct Mentions<'a> {
    engine: &'a BTreeSet<String>,
    found: Vec<String>,
}

impl Mentions<'_> {
    fn names(&self, name: &str) -> bool {
        name == ENGINE || self.engine.contains(name)
    }
}

impl<'ast> Visit<'ast> for Mentions<'_> {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        if let Some(first) = path.segments.first()
            && self.names(&first.ident.to_string())
        {
            let segments: Vec<String> = path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect();
            self.found.push(segments.join("::"));
        }
        visit::visit_path(self, path);
    }
}

fn public(visibility: &Visibility) -> bool {
    matches!(visibility, Visibility::Public(_))
}

fn item_leaks(item: &Item, types: &BTreeSet<String>, mentions: &mut Mentions) {
    match item {
        Item::Fn(item) if public(&item.vis) => mentions.visit_signature(&item.sig),
        Item::Struct(item) if public(&item.vis) => {
            mentions.visit_generics(&item.generics);
            item.fields
                .iter()
                .filter(|field| public(&field.vis))
                .for_each(|field| mentions.visit_type(&field.ty));
        }
        Item::Enum(item) if public(&item.vis) => {
            mentions.visit_generics(&item.generics);
            item.variants
                .iter()
                .flat_map(|variant| &variant.fields)
                .for_each(|field| mentions.visit_type(&field.ty));
        }
        Item::Type(item) if public(&item.vis) => mentions.visit_type(&item.ty),
        Item::Const(item) if public(&item.vis) => mentions.visit_type(&item.ty),
        Item::Static(item) if public(&item.vis) => mentions.visit_type(&item.ty),
        Item::Use(item) if public(&item.vis) => {
            let mut idents = Vec::new();
            use_idents(&item.tree, &mut idents);
            let reexported: Vec<String> = idents
                .into_iter()
                .filter(|ident| mentions.names(ident))
                .collect();
            mentions.found.extend(reexported);
        }
        Item::Impl(item) => impl_leaks(item, types, mentions),
        _ => {}
    }
}

/// An inherent impl exposes its public items; a trait impl for one of the
/// crate's public types exposes the trait, the type and every item.
fn impl_leaks(item: &ItemImpl, types: &BTreeSet<String>, mentions: &mut Mentions) {
    let exposed_trait = item
        .trait_
        .as_ref()
        .filter(|_| names_public_type(&item.self_ty, types));
    if let Some((_, path, _)) = exposed_trait {
        mentions.visit_generics(&item.generics);
        mentions.visit_path(path);
        mentions.visit_type(&item.self_ty);
    }
    let exposed = |visibility: &Visibility| exposed_trait.is_some() || public(visibility);
    for member in &item.items {
        match member {
            ImplItem::Fn(member) if exposed(&member.vis) => mentions.visit_signature(&member.sig),
            ImplItem::Const(member) if exposed(&member.vis) => mentions.visit_type(&member.ty),
            ImplItem::Type(member) if exposed(&member.vis) => mentions.visit_type(&member.ty),
            _ => {}
        }
    }
}

fn names_public_type(ty: &Type, types: &BTreeSet<String>) -> bool {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .is_some_and(|segment| types.contains(&segment.ident.to_string())),
        _ => false,
    }
}

/// The names of the crate's public structs and enums.
fn public_types(files: &[(String, File)]) -> BTreeSet<String> {
    files
        .iter()
        .flat_map(|(_, file)| &file.items)
        .filter_map(|item| match item {
            Item::Struct(item) if public(&item.vis) => Some(item.ident.to_string()),
            Item::Enum(item) if public(&item.vis) => Some(item.ident.to_string()),
            _ => None,
        })
        .collect()
}

/// The names of the crate's type aliases of engine types.
fn engine_aliases(files: &[(String, File)]) -> BTreeSet<String> {
    let mut aliases = BTreeSet::new();
    for (_, file) in files {
        let engine = engine_imports(file);
        for item in &file.items {
            if let Item::Type(alias) = item {
                let mut mentions = Mentions {
                    engine: &engine,
                    found: Vec::new(),
                };
                mentions.visit_type(&alias.ty);
                if !mentions.found.is_empty() {
                    aliases.insert(alias.ident.to_string());
                }
            }
        }
    }
    aliases
}

/// The engine paths, and glob imports of the engine, that each file's
/// public interface could name.
fn interface_leaks(files: &[(String, File)]) -> Vec<String> {
    let types = public_types(files);
    let aliases = engine_aliases(files);
    let mut leaks = Vec::new();
    for (path, file) in files {
        let engine: BTreeSet<String> = engine_imports(file).union(&aliases).cloned().collect();
        if engine.contains("*") {
            leaks.push(format!("{path}: a glob import of the engine"));
        }
        let mut mentions = Mentions {
            engine: &engine,
            found: Vec::new(),
        };
        for item in &file.items {
            item_leaks(item, &types, &mut mentions);
        }
        leaks.extend(mentions.found.iter().map(|leak| format!("{path}: {leak}")));
    }
    leaks
}

/// Every source file under `dir`, parsed.
fn parse(dir: &Path) -> Vec<(String, File)> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).expect("read source directory") {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            files.extend(parse(&path));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let source = fs::read_to_string(&path).expect("read source");
            let file = syn::parse_file(&source).expect("parse source");
            files.push((path.display().to_string(), file));
        }
    }
    files
}

fn parsed(sources: &[&str]) -> Vec<(String, File)> {
    sources
        .iter()
        .enumerate()
        .map(|(index, source)| (format!("{index}.rs"), syn::parse_file(source).unwrap()))
        .collect()
}

/// ADR 0007 (#83): nothing `climon2-vt` makes public, the read-only views a
/// monitor paints from included, names an engine type.
#[test]
fn the_virtual_terminal_interface_names_no_engine_type() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/climon2-vt/src");
    let files = parse(&source);
    assert!(
        files.iter().any(|(path, _)| path.ends_with("view.rs")),
        "{} holds the crate's sources",
        source.display()
    );
    let leaks = interface_leaks(&files);
    assert!(leaks.is_empty(), "{leaks:#?}");
}

#[test]
fn the_engine_type_guard_rejects_engine_types_in_the_interface() {
    for sources in [
        &["use alacritty_terminal::Grid; pub fn f() -> Grid { todo!() }"][..],
        &["pub fn f(term: &alacritty_terminal::Term<()>) {}"],
        &["use alacritty_terminal::term::cell::Cell as E; pub struct S { pub cell: E }"],
        &["use alacritty_terminal::term::{self}; pub struct S { pub mode: term::TermMode }"],
        &["pub enum E<'a> { A(&'a alacritty_terminal::Grid<u8>) }"],
        &["pub use alacritty_terminal::Term;"],
        &["use alacritty_terminal::Grid; pub use self::Grid as Exposed;"],
        &[
            "use alacritty_terminal::Grid; pub struct S; impl S { pub fn g(&self) -> &Grid { todo!() } }",
        ],
        &[
            "use alacritty_terminal::Grid; pub struct S; impl From<Grid> for S { fn from(_: Grid) -> Self { S } }",
        ],
        &[
            "use alacritty_terminal::Grid; pub struct S; impl Iterator for S { type Item = Grid; fn next(&mut self) -> Option<Grid> { None } }",
        ],
        &[
            "use alacritty_terminal::term::TermMode; pub(crate) type Entry = (TermMode, u8);",
            "use crate::a::Entry; pub fn h() -> Entry { todo!() }",
        ],
        &["use alacritty_terminal::*; pub fn f() {}"],
    ] {
        assert!(!interface_leaks(&parsed(sources)).is_empty(), "{sources:?}");
    }
}

#[test]
fn the_engine_type_guard_accepts_engine_types_kept_private() {
    let sources = [
        "use alacritty_terminal::{Grid, term::{TermMode, cell::Cell as E}};\n\
         pub struct S<'a> { grid: &'a Grid<E>, pub mode: u8 }\n\
         pub(crate) fn f(_: TermMode) {}\n\
         struct P; impl From<Grid> for P { fn from(_: Grid) -> Self { P } }\n\
         impl<'a> S<'a> { pub fn rows(&self) -> usize { 0 } pub(crate) fn grid(&self) -> &Grid<E> { self.grid } }\n\
         pub enum Screen { Normal, Alternate }\n\
         pub use crate::view::Cell;",
    ];
    assert_eq!(interface_leaks(&parsed(&sources)), Vec::<String>::new());
}
