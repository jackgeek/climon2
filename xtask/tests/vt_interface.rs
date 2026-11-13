//! ADR 0007: `climon2-vt` exposes only climon2-owned types. Its public items
//! (functions, struct fields, enum variants, traits, type aliases, constants,
//! statics and re-exports, inline modules' included) and the trait impls of
//! its public types never name a type of the engine, `alacritty_terminal`:
//! not by path, not by a name a `use` binds to one (renamed, through `self`
//! or through another module of the crate), and not through a crate type
//! alias of one, written plainly or as a `crate::`, `self::` or `super::`
//! path. A glob import of the engine fails the check outright.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
use syn::{
    File, ImplItem, Item, ItemImpl, ItemTrait, TraitItem, Type, UseTree, Visibility,
    visit::{self, Visit},
};

const ENGINE: &str = "alacritty_terminal";

/// One module of the crate, a file or an inline module, by its path from
/// the crate root.
struct Module {
    path: Vec<String>,
    source: String,
    items: Vec<Item>,
}

/// Every module of a crate whose files are given by module path.
fn modules(files: Vec<(Vec<String>, String, File)>) -> Vec<Module> {
    let mut modules = Vec::new();
    for (path, source, file) in files {
        nest(path, &source, file.items, &mut modules);
    }
    modules
}

/// A module and, as modules of their own, the inline modules it holds.
fn nest(path: Vec<String>, source: &str, mut items: Vec<Item>, modules: &mut Vec<Module>) {
    for item in &mut items {
        if let Item::Mod(module) = item
            && let Some((_, content)) = &mut module.content
        {
            let inner = [path.clone(), vec![module.ident.to_string()]].concat();
            nest(inner, source, std::mem::take(content), modules);
        }
    }
    modules.push(Module {
        path,
        source: source.to_owned(),
        items,
    });
}

/// Each name a `use` tree binds: the full path it names and its local name.
fn bindings(tree: &UseTree, prefix: &[String], out: &mut Vec<(Vec<String>, String)>) {
    let with = |name: String| [prefix, &[name]].concat();
    match tree {
        UseTree::Path(path) => bindings(&path.tree, &with(path.ident.to_string()), out),
        UseTree::Name(name) if name.ident == "self" => {
            let local = prefix.last().cloned().unwrap_or_default();
            out.push((prefix.to_vec(), local));
        }
        UseTree::Name(name) => out.push((with(name.ident.to_string()), name.ident.to_string())),
        UseTree::Rename(rename) => {
            out.push((with(rename.ident.to_string()), rename.rename.to_string()))
        }
        UseTree::Glob(_) => out.push((with("*".to_owned()), "*".to_owned())),
        UseTree::Group(group) => group
            .items
            .iter()
            .for_each(|item| bindings(item, prefix, out)),
    }
}

/// The names each module binds to engine items, by module path.
type Engine = BTreeMap<Vec<String>, BTreeSet<String>>;

/// The module a path's leading segments name, from `module`: `crate`,
/// `self`, `super` or a child module begin a path within the crate; `None`
/// for any other path.
fn resolve(engine: &Engine, module: &[String], segments: &[String]) -> Option<Vec<String>> {
    let mut at = match segments.first().map(String::as_str) {
        Some("crate") => Vec::new(),
        Some("self" | "super") => module.to_vec(),
        Some(child) => {
            let inner = [module, &[child.to_owned()]].concat();
            engine.contains_key(&inner).then_some(module.to_vec())?
        }
        None => return None,
    };
    let start = usize::from(matches!(segments[0].as_str(), "crate" | "self"));
    for segment in &segments[start..] {
        if segment == "super" {
            at.pop();
        } else {
            at.push(segment.clone());
        }
    }
    Some(at)
}

/// Whether `segments` name an engine item from `module`.
fn names_engine(engine: &Engine, module: &[String], segments: &[String]) -> bool {
    let Some((last, prefix)) = segments.split_last() else {
        return false;
    };
    if segments[0] == ENGINE {
        return true;
    }
    if prefix.is_empty() {
        return engine.get(module).is_some_and(|names| names.contains(last));
    }
    if engine
        .get(module)
        .is_some_and(|names| names.contains(&segments[0]))
    {
        return true;
    }
    resolve(engine, module, prefix)
        .and_then(|target| engine.get(&target))
        .is_some_and(|names| names.contains(last))
}

/// The paths in what it visits that name engine items, from one module.
struct Mentions<'a> {
    engine: &'a Engine,
    module: &'a [String],
    found: Vec<String>,
}

impl<'ast> Visit<'ast> for Mentions<'_> {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        let segments: Vec<String> = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect();
        if names_engine(self.engine, self.module, &segments) {
            self.found.push(segments.join("::"));
        }
        visit::visit_path(self, path);
    }
}

/// Engine names bound by each module's imports and type aliases, followed
/// through `use` items and aliases elsewhere in the crate.
fn engine_names(modules: &[Module]) -> Engine {
    let mut engine: Engine = modules
        .iter()
        .map(|module| (module.path.clone(), BTreeSet::new()))
        .collect();
    loop {
        let mut grown = engine.clone();
        for module in modules {
            let names = grown.get_mut(&module.path).expect("every module is known");
            for item in &module.items {
                match item {
                    Item::Use(item) => {
                        let mut bound = Vec::new();
                        bindings(&item.tree, &[], &mut bound);
                        names.extend(
                            bound
                                .into_iter()
                                .filter(|(path, _)| names_engine(&engine, &module.path, path))
                                .map(|(_, local)| local),
                        );
                    }
                    Item::Type(alias) => {
                        let mut mentions = Mentions {
                            engine: &engine,
                            module: &module.path,
                            found: Vec::new(),
                        };
                        mentions.visit_type(&alias.ty);
                        if !mentions.found.is_empty() {
                            names.insert(alias.ident.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
        if grown == engine {
            return engine;
        }
        engine = grown;
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
        Item::Trait(item) if public(&item.vis) => trait_leaks(item, mentions),
        Item::Type(item) if public(&item.vis) => mentions.visit_type(&item.ty),
        Item::Const(item) if public(&item.vis) => mentions.visit_type(&item.ty),
        Item::Static(item) if public(&item.vis) => mentions.visit_type(&item.ty),
        Item::Use(item) if public(&item.vis) => {
            let mut bound = Vec::new();
            bindings(&item.tree, &[], &mut bound);
            for (path, _) in bound {
                if names_engine(mentions.engine, mentions.module, &path) {
                    mentions.found.push(path.join("::"));
                }
            }
        }
        Item::Impl(item) => impl_leaks(item, types, mentions),
        _ => {}
    }
}

/// A public trait exposes its generics, supertraits and every item.
fn trait_leaks(item: &ItemTrait, mentions: &mut Mentions) {
    mentions.visit_generics(&item.generics);
    item.supertraits
        .iter()
        .for_each(|bound| mentions.visit_type_param_bound(bound));
    for member in &item.items {
        match member {
            TraitItem::Fn(member) => mentions.visit_signature(&member.sig),
            TraitItem::Const(member) => mentions.visit_type(&member.ty),
            TraitItem::Type(member) => mentions.visit_trait_item_type(member),
            _ => {}
        }
    }
}

/// An inherent impl exposes its public items; a trait impl for one of the
/// crate's public types, or a reference to one, exposes the trait, the type
/// and every item.
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
        Type::Reference(reference) => names_public_type(&reference.elem, types),
        Type::Paren(inner) => names_public_type(&inner.elem, types),
        Type::Group(inner) => names_public_type(&inner.elem, types),
        _ => false,
    }
}

/// The names of the crate's public structs, enums and traits.
fn public_types(modules: &[Module]) -> BTreeSet<String> {
    modules
        .iter()
        .flat_map(|module| &module.items)
        .filter_map(|item| match item {
            Item::Struct(item) if public(&item.vis) => Some(item.ident.to_string()),
            Item::Enum(item) if public(&item.vis) => Some(item.ident.to_string()),
            Item::Trait(item) if public(&item.vis) => Some(item.ident.to_string()),
            _ => None,
        })
        .collect()
}

/// The engine names, and glob imports of the engine, that each module's
/// public items could expose.
fn interface_leaks(files: Vec<(Vec<String>, String, File)>) -> Vec<String> {
    let modules = modules(files);
    let engine = engine_names(&modules);
    let types = public_types(&modules);
    let mut leaks = Vec::new();
    for module in &modules {
        let label = format!("{} ({})", module.path.join("::"), module.source);
        if engine[&module.path].contains("*") {
            leaks.push(format!("{label}: a glob import of the engine"));
        }
        let mut mentions = Mentions {
            engine: &engine,
            module: &module.path,
            found: Vec::new(),
        };
        for item in &module.items {
            item_leaks(item, &types, &mut mentions);
        }
        leaks.extend(mentions.found.iter().map(|leak| format!("{label}: {leak}")));
    }
    leaks
}

/// Every source file under `dir`, parsed, by its module path from `root`.
fn parse(root: &Path, dir: &Path) -> Vec<(Vec<String>, String, File)> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).expect("read source directory") {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            files.extend(parse(root, &path));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let relative = path.strip_prefix(root).unwrap().with_extension("");
            let module: Vec<String> = relative
                .iter()
                .map(|part| part.to_string_lossy().into_owned())
                .filter(|part| part != "lib" && part != "mod")
                .collect();
            let source = fs::read_to_string(&path).expect("read source");
            let file = syn::parse_file(&source).expect("parse source");
            files.push((module, path.display().to_string(), file));
        }
    }
    files
}

/// Sources by module name; `lib` is the crate root.
fn parsed(sources: &[(&str, &str)]) -> Vec<(Vec<String>, String, File)> {
    sources
        .iter()
        .map(|(module, source)| {
            let path = if *module == "lib" {
                Vec::new()
            } else {
                module.split("::").map(str::to_owned).collect()
            };
            (path, (*module).to_owned(), syn::parse_file(source).unwrap())
        })
        .collect()
}

/// ADR 0007 (#83): nothing `climon2-vt` makes public, the read-only views a
/// monitor paints from included, names an engine type.
#[test]
fn the_virtual_terminal_interface_names_no_engine_type() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/climon2-vt/src");
    let files = parse(&source, &source);
    assert!(
        files.iter().any(|(module, _, _)| module == &["view"]),
        "{} holds the crate's sources",
        source.display()
    );
    let leaks = interface_leaks(files);
    assert!(leaks.is_empty(), "{leaks:#?}");
}

/// ADR 0007 (#83): the check above fails for each way a public item can
/// expose an engine type, so its passing proves something.
#[test]
fn the_engine_type_guard_rejects_engine_types_in_the_interface() {
    for sources in [
        &[(
            "lib",
            "use alacritty_terminal::Grid; pub fn f() -> Grid { todo!() }",
        )][..],
        &[("lib", "pub fn f(term: &alacritty_terminal::Term<()>) {}")],
        &[(
            "lib",
            "use alacritty_terminal::term::cell::Cell as E; pub struct S { pub cell: E }",
        )],
        &[(
            "lib",
            "use alacritty_terminal::term::{self}; pub struct S { pub mode: term::TermMode }",
        )],
        &[(
            "lib",
            "pub enum E<'a> { A(&'a alacritty_terminal::Grid<u8>) }",
        )],
        &[("lib", "pub use alacritty_terminal::Term;")],
        &[(
            "lib",
            "use alacritty_terminal::Grid; pub use self::Grid as Exposed;",
        )],
        &[(
            "lib",
            "use alacritty_terminal::Grid; pub struct S; impl S { pub fn g(&self) -> &Grid { todo!() } }",
        )],
        &[(
            "lib",
            "use alacritty_terminal::Grid; pub struct S; impl From<Grid> for S { fn from(_: Grid) -> Self { S } }",
        )],
        &[(
            "lib",
            "use alacritty_terminal::Grid; pub struct S; impl Iterator for S { type Item = Grid; fn next(&mut self) -> Option<Grid> { None } }",
        )],
        &[(
            "lib",
            "use alacritty_terminal::term::cell::Cell as E; pub struct Rows; impl IntoIterator for &Rows { type Item = E; type IntoIter = std::vec::IntoIter<E>; fn into_iter(self) -> Self::IntoIter { todo!() } }",
        )],
        &[(
            "lib",
            "use alacritty_terminal::Grid; pub trait T { fn grid(&self) -> &Grid<u8>; }",
        )],
        &[(
            "lib",
            "pub trait T: alacritty_terminal::event::EventListener {}",
        )],
        &[(
            "lib",
            "use alacritty_terminal::Grid; pub trait T { type Screen: AsRef<Grid<u8>>; }",
        )],
        &[(
            "lib",
            "pub mod inner { use alacritty_terminal::Grid; pub fn f() -> Grid<u8> { todo!() } }",
        )],
        &[(
            "lib",
            "mod inner { pub struct S { pub grid: alacritty_terminal::Grid<u8> } } pub use inner::S;",
        )],
        &[
            (
                "a",
                "use alacritty_terminal::term::TermMode; pub(crate) type Entry = (TermMode, u8);",
            ),
            ("b", "use crate::a::Entry; pub fn h() -> Entry { todo!() }"),
        ],
        &[
            (
                "a",
                "use alacritty_terminal::term::TermMode; pub(crate) type Entry = (TermMode, u8);",
            ),
            ("b", "pub fn h() -> crate::a::Entry { todo!() }"),
        ],
        &[
            ("a", "pub(crate) use alacritty_terminal::Grid;"),
            ("a::b", "pub fn h() -> super::Grid<u8> { todo!() }"),
        ],
        &[
            ("a", "pub(crate) use alacritty_terminal::Grid;"),
            (
                "b",
                "use crate::a::Grid as Hidden; pub(crate) type Alias = Hidden<u8>;",
            ),
            ("c", "pub use crate::b::Alias;"),
        ],
        &[("lib", "use alacritty_terminal::*; pub fn f() {}")],
    ] {
        assert!(!interface_leaks(parsed(sources)).is_empty(), "{sources:?}");
    }
}

/// ADR 0007 (#83): engine types the crate keeps private, and its own types
/// that share an engine type's name, pass the check.
#[test]
fn the_engine_type_guard_accepts_engine_types_kept_private() {
    let sources = [
        (
            "lib",
            "pub mod view; mod render;\n\
             use alacritty_terminal::{Grid, term::{TermMode, cell::Cell as E}};\n\
             pub struct S<'a> { grid: &'a Grid<E>, pub mode: u8 }\n\
             pub(crate) fn f(_: TermMode) {}\n\
             struct P; impl From<Grid> for P { fn from(_: Grid) -> Self { P } }\n\
             impl<'a> S<'a> { pub fn rows(&self) -> usize { 0 } pub(crate) fn grid(&self) -> &Grid<E> { self.grid } }\n\
             pub enum Screen { Normal, Alternate }\n\
             impl std::fmt::Debug for Screen { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { Ok(()) } }\n\
             mod private { pub(crate) fn g(_: alacritty_terminal::Grid<u8>) {} }\n\
             pub use crate::view::Cell;",
        ),
        (
            "render",
            "use alacritty_terminal::term::cell::Cell; pub(crate) fn cells(_: &[Cell]) {}",
        ),
        (
            "view",
            "pub struct Cell; pub fn cell() -> Cell { Cell } pub fn same() -> crate::view::Cell { Cell }",
        ),
    ];
    assert_eq!(interface_leaks(parsed(&sources)), Vec::<String>::new());
}
