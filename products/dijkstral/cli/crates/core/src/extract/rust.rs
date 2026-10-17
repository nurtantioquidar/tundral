//! Rust (§11) extraction. See `docs/graphify-format.md`.
//!
//! One cheap item-level pre-scan collects the struct/enum/trait names a file declares (type
//! references and impl owners resolve against them even when declared further down), then one walk
//! emits nodes and structural edges in source order and records call sites. Calls are resolved at
//! the end of the file, once every function, method, variant and macro is known; bare calls left
//! unresolved go to the cross-file pass (`crate::resolve::rust`) through [`RustFacts`], and so do
//! every path-qualified call, every `self.field.m()` call, every struct field's declared type
//! (module and field-type resolution, Q23), every function's declared return type and the
//! symbolic receiver type of every other method call ([`infer`], local type inference).
//!
//! Deliberate departures from Graphify 0.9.86 (quirks in §15 not reproduced):
//!
//! - Q14: references to a generic type parameter in scope (`fn f<W: Write>(w: W)`) are not
//!   emitted, so no `t`/`w` stub nodes exist.
//! - Q15: Python builtin names (`len`, `str`, `int`…) resolve for path-qualified in-file calls
//!   (`convert::str(..)`, `Type::len(..)`), which no local binding can shadow; bare and
//!   method-syntax calls and the cross-file pass keep the denylist. Rust stubs never resolve to
//!   definitions in non-Rust files.
//! - `Enum::Variant(..)` for an enum declared in the same file resolves to that enum's variant,
//!   denylisted names included (`WalkEvent::File`). Graphify takes the file's last node labelled
//!   `Variant` instead (`MatchStrategy::Literal` → `Token::Literal`).
//! - A type name resolves in-file only to a struct, enum or trait the file declares. Graphify also
//!   accepts any earlier node of the file with the id `<file>_<name>`, so `fn seq() -> Seq` loses
//!   its edge and later `-> Seq` signatures point at the function `seq`.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use tree_sitter::{Node as TsNode, Tree};

use super::{slug, Ctx};
use crate::facts::{Attr, LangFacts};

mod infer;

/// Facts the cross-file pass (`crate::resolve::rust`) needs from one file. Ids are local (before
/// collision renaming).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RustFacts {
    /// `use` leaf ids that need a shared external node, in first-use order. A leaf whose id is also
    /// a type-reference stub of this file is left out: the stub shape wins (§11.2 rule 1).
    pub externals: Vec<String>,
    /// Bare calls not resolved in-file, in call order (§11.5 cross-file pass).
    pub calls: Vec<PendingCall>,
    /// `self.name()` calls inside an impl when the file has no method `name` at all: they resolve
    /// to the method `name` of the same type in another file (EXTRACTED, score 1.0).
    pub self_calls: Vec<SelfCall>,
    /// Every `use` leaf with its full path, function bodies included (module resolution).
    pub uses: Vec<RsUse>,
    /// `mod name;` declarations (out-of-line modules).
    pub mods: Vec<RsMod>,
    /// Names of the items the file declares, inline modules included and flattened, each with
    /// whether a path can continue through it (a module or a type, not a function or value).
    pub items: Vec<(String, bool)>,
    /// `extern crate name [as alias];`: (crate name, bound name).
    pub extern_crates: Vec<(String, String)>,
    /// Path-qualified calls (`a::b::f(..)`, `Type::f(..)`), in call order, for module
    /// resolution; one per (caller, path) at its first site.
    pub path_calls: Vec<PathCall>,
    /// `self.a.b.m(..)` calls inside an impl, in call order, for field-type resolution.
    pub field_calls: Vec<FieldCall>,
    /// Struct fields whose declared type is a path, in source order.
    pub fields: Vec<RsField>,
    /// The type path of each impl owner node the file creates (one per owner, first impl).
    pub impls: Vec<RsImpl>,
    /// Names of the traits the file declares (inline modules included).
    pub traits: Vec<String>,
    /// Names of the inline `mod name { … }` blocks (flattened into the file).
    pub inline_mods: Vec<String>,
    /// `type Name = a::B<..>;` aliases of a path type.
    pub aliases: Vec<RsAlias>,
    /// Symbolic types of method-call receivers (local type inference, [`infer`]); the
    /// [`MethodCall`]s and other entries index into it.
    pub tys: Vec<RsTy>,
    /// `recv.m(..)` calls whose receiver has a symbolic type, other than `self.m()` and
    /// `self.a.b.m()` (those are [`SelfCall`]s and [`FieldCall`]s), in call order.
    pub method_calls: Vec<MethodCall>,
    /// Declared return types of the file's functions and methods.
    pub returns: Vec<RsReturn>,
}

/// A type as written: path segments (generic arguments dropped; `Self` replaced by the impl's
/// type), whether `Box`, `Arc` or `Rc` wraps it, and its first generic argument (the `T` of
/// `Option<T>` and `Result<T, E>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RsPathTy {
    pub path: Vec<String>,
    pub wrapped: bool,
    pub inner: Option<Box<RsPathTy>>,
}

/// The symbolic type of an expression in a function body, as far as a declaration fixes it. The
/// cross-file pass evaluates it against the repository's declarations; anything else (generic
/// parameters, trait objects, closures, patterns) has no `RsTy` at all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RsTy {
    /// A type written in the body or the signature (`let x: T`, `x: &T`, `T { .. }`, `self`),
    /// resolved where the call is.
    Written(RsPathTy),
    /// What `a::b::f(..)` / `T::f(..)` returns (a tuple struct or variant constructs its type).
    PathRet { qual: Vec<String>, name: String },
    /// What a bare `f(..)` returns; the name is no local binding.
    BareRet(String),
    /// What `recv.name(..)` returns.
    MethodRet { recv: u32, name: String },
    /// The declared type of the field `recv.name`.
    Field { recv: u32, name: String },
    /// `recv?`: the `T` of an `Option<T>` / `Result<T, E>`.
    Unwrap(u32),
    /// `recv.await` on what an `async fn` returns.
    Await(u32),
    /// `Box::new(x)`, `Arc::new(x)`, `Rc::new(x)`: `x` behind a pointer.
    Wrap(u32),
}

/// `recv.name(..)` with a typed receiver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MethodCall {
    /// Local id of the calling function or method.
    pub caller: String,
    /// Index of the receiver's type in [`RustFacts::tys`].
    pub recv: u32,
    pub name: String,
    /// Enclosing inline `mod` blocks at the call site.
    pub depth: u32,
    pub line: u32,
    /// Local id of the node in-file resolution chose, if any.
    pub in_file: Option<String>,
}

/// The declared return type of a function or method whose node the build keeps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RsReturn {
    /// Local id of the function or method.
    pub id: String,
    /// Enclosing inline `mod` blocks.
    pub depth: u32,
    pub ty: RsPathTy,
    /// An `async fn`: callers get a future, and `.await` yields `ty`.
    pub is_async: bool,
}

/// `type name = ty;` where `ty` is a path (generic arguments dropped).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RsAlias {
    pub name: String,
    /// Enclosing inline `mod` blocks.
    pub depth: u32,
    pub ty: Vec<String>,
}

/// `impl [Trait for] a::B<..>` whose owner node is not a type the file declares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RsImpl {
    /// Local id of the owner node.
    pub owner: String,
    /// The implemented type's path segments, generic arguments dropped.
    pub ty: Vec<String>,
    /// Enclosing inline `mod` blocks at the impl.
    pub depth: u32,
}

/// A path-qualified call site.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathCall {
    /// Local id of the calling function or method.
    pub caller: String,
    /// Qualifier segments as written (`""` first for a leading `::`); a lone `Self` is replaced
    /// by the impl's type name.
    pub qual: Vec<String>,
    pub name: String,
    /// Enclosing inline `mod` blocks at the call site.
    pub depth: u32,
    pub line: u32,
    /// Local id of the node in-file resolution chose, if any.
    pub in_file: Option<String>,
}

/// `self.<fields>.name(..)` inside an impl.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldCall {
    pub caller: String,
    /// The impl's type name (`SearchWorker` for `impl<W> SearchWorker<W>`).
    pub self_ty: String,
    /// Enclosing inline `mod` blocks at the impl.
    pub depth: u32,
    /// The field chain after `self` (`["config", "globs"]`; tuple fields as `"0"`).
    pub fields: Vec<String>,
    pub name: String,
    pub line: u32,
    /// Local id of the node in-file resolution chose, if any.
    pub in_file: Option<String>,
}

/// A struct field declared with a path type: `name: a::B<..>`, also through `&`, `&mut`,
/// `Box<..>`, `Arc<..>` and `Rc<..>` (`wrapped`). Generic parameters are left out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RsField {
    /// The struct's name.
    pub owner: String,
    /// Enclosing inline `mod` blocks at the struct.
    pub depth: u32,
    /// Field name; tuple fields are `"0"`, `"1"`…
    pub name: String,
    /// The type's path segments, generic arguments dropped.
    pub ty: Vec<String>,
    /// The type sits inside `Box`, `Arc` or `Rc`, whose own trait methods come first.
    pub wrapped: bool,
    /// The first generic argument as written (`Option<T>`), for local type inference.
    pub inner: Option<Box<RsPathTy>>,
}

/// One leaf of a `use` tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RsUse {
    /// Segments as written (`crate`, `self`, `super`, names); `""` first for a leading `::`. A
    /// glob's path names the module; `a::{self}` names `a`.
    pub path: Vec<String>,
    /// The name the leaf binds (the alias, else the last segment); `None` for globs and `as _`.
    pub bound: Option<String>,
    pub glob: bool,
    /// Enclosing inline `mod` blocks at the use site: `super` inside one stays in the file.
    pub depth: u32,
    /// Inside a function body (Graphify emits no edge for it).
    pub in_fn: bool,
    /// Local id of that function: the binding is visible only in its body.
    pub func: Option<String>,
    pub line: u32,
}

/// `mod name;` inside the inline modules `inline`, with its `#[path = "…"]` if any.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RsMod {
    pub inline: Vec<String>,
    pub name: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelfCall {
    /// Local id of the calling method.
    pub caller: String,
    /// The impl's type key: the owner node's `_rust_impl_key`, else its label.
    pub owner_key: String,
    pub name: String,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingCall {
    /// Local id of the calling function or method.
    pub caller: String,
    pub name: String,
    pub line: u32,
    /// On the cross-file denylist (§11.5): resolved only through an import of the name.
    pub common: bool,
    /// The caller's definition is the node the build keeps for its id: only then does an edge
    /// that Graphify would not infer start from it.
    pub kept: bool,
}

/// Type names never referenced (§11.4); also the std-trait denylist for `implements` and
/// `inherits`, which probing showed to be the same list. A type the file declares itself is
/// referenced even when its name is listed (`struct Error`).
const TYPE_DENY: &str = "String Vec Option Result Error Box HashMap HashSet BTreeMap BTreeSet
    VecDeque BinaryHeap Cow Path PathBuf Arc Rc RefCell Cell Weak Pin Duration Instant SystemTime
    OsStr OsString CString CStr Ordering PhantomData Range RangeInclusive RangeFrom RangeTo
    RangeFull Self Iterator Debug Display Clone Default Send Sync Hash Eq PartialEq Ord PartialOrd
    str Sized Fn FnMut FnOnce Future Into From AsRef AsMut TryFrom TryInto IntoIterator Extend Copy
    Drop Deref DerefMut ManuallyDrop NonZeroUsize NonZeroU8 NonZeroU16 NonZeroU32 NonZeroU64 Ok
    Err Some None";

/// Call names never resolved in a Rust file, case-sensitive (§11.5): Python builtins.
const CALL_DENY: &str = "abs all any bool bytes callable delattr dict dir enumerate filter float
    getattr hasattr hash id int isinstance iter len list map max min next open print range repr
    reversed round set setattr sorted str sum tuple vars zip";

/// Constructor names (`Name(..)`) never resolved: std, JS and Python type names.
const CTOR_DENY: &str = "Path PathBuf Map Set String Ok Some Err None Box Vec Hash Instant
    Duration SystemTime Default Clone Option Result Error Arc Rc Cell RefCell Display Debug From
    Into Send Int Float Bool Boolean Number Double Decimal Range Object Array Date Data HashMap
    HashSet BTreeMap BTreeSet VecDeque BinaryHeap Cow File Ordering PhantomData Weak Pin OsString
    OsStr CString CStr Promise RegExp Symbol BigInt Character Math JSON Response Request Headers
    URL URLSearchParams FormData Blob Proxy Reflect WeakMap WeakSet Intl UUID Uuid Color Font";

/// Names the cross-file pass never resolves, compared lowercased (§11.5), on top of the lists
/// above.
const CROSS_FILE_DENY: &str = "build new run get set init parse from into len default clone start
    read write send push pop remove insert filter map next iter to_string is_empty contains hash
    unwrap expect ok err some none lock to_owned now collect from_str";

fn word_set(words: &'static str) -> HashSet<&'static str> {
    words.split_whitespace().collect()
}

static TYPE_DENY_SET: LazyLock<HashSet<&str>> = LazyLock::new(|| word_set(TYPE_DENY));
static CALL_DENY_SET: LazyLock<HashSet<&str>> = LazyLock::new(|| word_set(CALL_DENY));
static CTOR_DENY_SET: LazyLock<HashSet<&str>> = LazyLock::new(|| word_set(CTOR_DENY));
static CROSS_FILE_DENY_SET: LazyLock<HashSet<&str>> = LazyLock::new(|| word_set(CROSS_FILE_DENY));

pub fn type_denied(name: &str) -> bool {
    TYPE_DENY_SET.contains(name)
}

pub fn call_denied(name: &str) -> bool {
    CALL_DENY_SET.contains(name)
}

pub fn ctor_denied(name: &str) -> bool {
    CTOR_DENY_SET.contains(name)
}

pub fn cross_file_denied(name: &str) -> bool {
    call_denied(name)
        || ctor_denied(name)
        || CROSS_FILE_DENY_SET.contains(name.to_ascii_lowercase().as_str())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RefCtx {
    Param,
    Return,
    Field,
}

impl RefCtx {
    fn as_str(self, nested: bool) -> &'static str {
        if nested {
            return "generic_arg";
        }
        match self {
            RefCtx::Param => "parameter_type",
            RefCtx::Return => "return_type",
            RefCtx::Field => "field",
        }
    }
}

/// The impl a method body belongs to.
#[derive(Clone)]
struct ImplScope<'a> {
    /// Owner node id.
    owner: String,
    /// The impl's type name (`Self`), `""` when the type is not a plain path.
    self_ty: &'a str,
    /// The owner node's `_rust_impl_key`, else its label (§11.5 cross-file `self.m()`).
    key: String,
}

/// How a call names its callee.
enum Callee<'a> {
    /// `name(..)`.
    Bare(&'a str),
    /// `module::name(..)` (lowercase qualifier, `crate::`, `super::`…): in-file only.
    Module(&'a str),
    /// `Type::name(..)`; `Self` already replaced by the impl's type name.
    Typed(&'a str, &'a str),
    /// `recv.name(..)`; the flag is set when the receiver is `self`.
    Method(&'a str, bool),
    /// `name!(..)`.
    Macro(&'a str),
}

/// Call sites left for the cross-file pass.
struct Pending {
    calls: Vec<PendingCall>,
    self_calls: Vec<SelfCall>,
    path_calls: Vec<PathCall>,
    field_calls: Vec<FieldCall>,
}

struct Site<'a> {
    caller: usize,
    callee: Callee<'a>,
    line: u32,
    /// Qualifier segments of a path callee (`a::b` of `a::b::f`), else empty.
    qual: Vec<&'a str>,
    /// Field chain of a `self.a.b.m()` receiver, else empty.
    fields: Vec<&'a str>,
    /// Enclosing inline `mod` blocks.
    depth: u32,
}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(vec![]);
    let Some(tree) = tree else {
        cx.facts.lang = LangFacts::Rust(RustFacts::default());
        return;
    };
    let root = tree.root_node();
    let mut w = Walker::new(cx);
    w.prescan(root);
    w.items(root);
    w.finish();
}

struct Walker<'c, 'a> {
    cx: &'c mut Ctx<'a>,
    file: String,
    /// Declared struct/enum/trait name → id (pre-scan), and the number of declarations per id.
    declared: HashMap<&'a str, String>,
    decl_count: HashMap<String, i64>,
    enums: HashSet<&'a str>,
    emitted: HashSet<String>,
    stubs: HashSet<String>,
    imports: Vec<String>,
    /// Generic type parameter names in scope (Q14: never referenced).
    generics: Vec<&'a str>,
    free_fns: HashMap<&'a str, String>,
    /// Method nodes by name, last definition wins (trait declarations included), for
    /// `recv.name()`. A method whose id an earlier node already holds is not a candidate.
    methods: HashMap<&'a str, String>,
    /// Impl methods by (owner id, name): `self.name()` inside an impl prefers its own owner.
    owner_methods: HashMap<(String, &'a str), String>,
    /// Impl methods by (type name, method name) (distinct ids), for `Type::name()`.
    type_methods: HashMap<(&'a str, &'a str), Vec<String>>,
    variants: HashMap<(&'a str, &'a str), String>,
    /// Ids whose kept node is an enum variant: a call resolved to one is a constructor.
    variant_ids: HashSet<String>,
    macros: HashMap<&'a str, String>,
    /// (caller id, enclosing impl).
    callers: Vec<(String, Option<ImplScope<'a>>)>,
    /// Index of the first node with each id.
    first_node: HashMap<String, usize>,
    sites: Vec<Site<'a>>,
    /// Names of the inline `mod` blocks around the current item.
    mod_path: Vec<&'a str>,
    uses: Vec<RsUse>,
    mods: Vec<RsMod>,
    items: Vec<(String, bool)>,
    item_seen: HashMap<&'a str, usize>,
    extern_crates: Vec<(String, String)>,
    /// Recording the `use` declarations of a function body.
    in_body: bool,
    /// The function whose body's `use` declarations are being recorded.
    body_fn: Option<String>,
    /// Per caller index: its definition is the node the build keeps for its id (not a later
    /// `#[cfg]` twin, a same-named method of another impl, or a fn sharing a struct's id).
    caller_kept: Vec<bool>,
    fields: Vec<RsField>,
    impls: Vec<RsImpl>,
    traits: Vec<String>,
    inline_mods: Vec<String>,
    aliases: Vec<RsAlias>,
    /// The impl's (or struct's) type path while its body is walked: what `Self` names.
    self_path: Vec<&'a str>,
    tys: Vec<RsTy>,
    method_calls_raw: Vec<infer::RawMethodCall<'a>>,
    returns: Vec<RsReturn>,
}

fn push_distinct(v: &mut Vec<String>, id: &str) {
    if !v.iter().any(|x| x == id) {
        v.push(id.to_string());
    }
}

fn named_children<'t>(n: TsNode<'t>) -> impl Iterator<Item = TsNode<'t>> {
    (0..n.named_child_count()).filter_map(move |i| n.named_child(i))
}

fn starts_upper(s: &str) -> bool {
    s.chars().next().is_some_and(char::is_uppercase)
}

impl<'c, 'a> Walker<'c, 'a> {
    fn new(cx: &'c mut Ctx<'a>) -> Self {
        let file = cx.file_id.clone();
        let mut emitted = HashSet::new();
        emitted.insert(file.clone());
        Walker {
            cx,
            file,
            declared: HashMap::new(),
            decl_count: HashMap::new(),
            enums: HashSet::new(),
            emitted,
            stubs: HashSet::new(),
            imports: Vec::new(),
            generics: Vec::new(),
            free_fns: HashMap::new(),
            methods: HashMap::new(),
            owner_methods: HashMap::new(),
            type_methods: HashMap::new(),
            variants: HashMap::new(),
            variant_ids: HashSet::new(),
            macros: HashMap::new(),
            callers: Vec::new(),
            first_node: HashMap::new(),
            sites: Vec::new(),
            mod_path: Vec::new(),
            uses: Vec::new(),
            mods: Vec::new(),
            items: Vec::new(),
            item_seen: HashMap::new(),
            extern_crates: Vec::new(),
            in_body: false,
            body_fn: None,
            caller_kept: Vec::new(),
            fields: Vec::new(),
            impls: Vec::new(),
            traits: Vec::new(),
            inline_mods: Vec::new(),
            aliases: Vec::new(),
            self_path: Vec::new(),
            tys: Vec::new(),
            method_calls_raw: Vec::new(),
            returns: Vec::new(),
        }
    }

    fn text(&self, n: TsNode) -> &'a str {
        self.cx.text(n)
    }

    fn field_text(&self, n: TsNode, field: &str) -> Option<&'a str> {
        n.child_by_field_name(field).map(|c| self.text(c))
    }

    // ---- pre-scan -------------------------------------------------------------------------

    /// Item-level only: struct/enum/trait declarations at file level and in inline modules.
    fn prescan(&mut self, container: TsNode) {
        for item in named_children(container) {
            match item.kind() {
                "struct_item" | "enum_item" | "trait_item" => {
                    let Some(name) = self.field_text(item, "name") else {
                        continue;
                    };
                    let id = Ctx::child_id(&self.file, name);
                    if item.kind() == "enum_item" {
                        self.enums.insert(name);
                    }
                    if item.kind() == "trait_item" && !self.traits.iter().any(|t| t == name) {
                        self.traits.push(name.to_string());
                    }
                    self.declared.entry(name).or_insert_with(|| id.clone());
                    *self.decl_count.entry(id).or_default() += 1;
                }
                "mod_item" => {
                    if let Some(body) = item.child_by_field_name("body") {
                        self.prescan(body);
                    }
                }
                _ => {}
            }
        }
    }

    // ---- emission helpers ----------------------------------------------------------------

    /// Emit a definition; true when it is the first node with this id (the one the build keeps).
    fn def(&mut self, id: &str, label: String, line: u32, extra: Vec<(String, Attr)>) -> bool {
        let first = self.emitted.insert(id.to_string());
        if first {
            self.first_node
                .insert(id.to_string(), self.cx.facts.nodes.len());
        }
        self.cx.add_def(id.to_string(), label, line, extra);
        first
    }

    fn edge(&mut self, source: &str, target: &str, relation: &str, line: u32, ctx: Option<&str>) {
        self.cx.add_edge(source, target, relation, line, ctx);
    }

    /// The node a type name refers to: the file's own struct/enum/trait, else a per-file stub
    /// (emitted at its first reference).
    fn type_target(&mut self, name: &str) -> Option<String> {
        if let Some(id) = self.declared.get(name) {
            return Some(id.clone());
        }
        let id = slug(name);
        if id.is_empty() {
            return None;
        }
        if self.stubs.insert(id.clone()) {
            self.emitted.insert(id.clone());
            self.cx.add_stub(id.clone(), name.to_string());
        }
        Some(id)
    }

    fn push_generics(&mut self, params: Option<TsNode>) -> usize {
        let before = self.generics.len();
        if let Some(params) = params {
            for p in named_children(params) {
                if p.kind() == "type_parameter" {
                    if let Some(name) = self.field_text(p, "name") {
                        self.generics.push(name);
                    }
                }
            }
        }
        before
    }

    /// Collect type names in a type node, pre-order, with whether each sits inside `<...>`.
    fn type_names(&self, n: TsNode, nested: bool, out: &mut Vec<(&'a str, bool)>) {
        match n.kind() {
            "type_identifier" => self.push_type(self.text(n), nested, out),
            "scoped_type_identifier" => {
                if let Some(name) = n.child_by_field_name("name") {
                    self.push_type(self.text(name), nested, out);
                }
            }
            "generic_type" => {
                if let Some(t) = n.child_by_field_name("type") {
                    match t.kind() {
                        "type_identifier" | "identifier" => {
                            self.push_type(self.text(t), nested, out)
                        }
                        "scoped_type_identifier" | "scoped_identifier" => {
                            if let Some(name) = t.child_by_field_name("name") {
                                self.push_type(self.text(name), nested, out);
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(args) = n.child_by_field_name("type_arguments") {
                    for a in named_children(args) {
                        self.type_names(a, true, out);
                    }
                }
            }
            "type_binding" => {
                if let Some(name) = n.child_by_field_name("name") {
                    self.push_type(self.text(name), true, out);
                }
                if let Some(t) = n.child_by_field_name("type") {
                    self.type_names(t, true, out);
                }
            }
            "array_type" => {
                if let Some(t) = n.child_by_field_name("element") {
                    self.type_names(t, nested, out);
                }
            }
            "lifetime" | "primitive_type" | "macro_invocation" | "type_parameters"
            | "where_clause" | "for_lifetimes" | "function_modifiers" => {}
            _ => {
                for c in named_children(n) {
                    self.type_names(c, nested, out);
                }
            }
        }
    }

    fn push_type(&self, name: &'a str, nested: bool, out: &mut Vec<(&'a str, bool)>) {
        let declared = self.declared.contains_key(name);
        if !self.generics.contains(&name) && (declared || !type_denied(name)) {
            out.push((name, nested));
        }
    }

    /// `references` edges from `source` for every type name in `ty`.
    fn type_refs(&mut self, source: &str, ty: Option<TsNode>, ctx: RefCtx, line: u32) {
        let Some(ty) = ty else { return };
        let mut names = Vec::new();
        self.type_names(ty, false, &mut names);
        for (name, nested) in names {
            // A type never references itself (`struct D { d: other::D }`).
            if let Some(target) = self.type_target(name).filter(|t| t != source) {
                self.edge(
                    source,
                    &target,
                    "references",
                    line,
                    Some(ctx.as_str(nested)),
                );
            }
        }
    }

    /// A trait bound or implemented trait: the first named type gets `relation`, the rest
    /// `references` (`generic_arg`).
    fn bound_refs(&mut self, source: &str, bound: TsNode, relation: &str, line: u32) {
        let mut names = Vec::new();
        self.type_names(bound, false, &mut names);
        for (i, (name, _)) in names.into_iter().enumerate() {
            if let Some(target) = self.type_target(name).filter(|t| t != source) {
                if i == 0 {
                    self.edge(source, &target, relation, line, None);
                } else {
                    self.edge(source, &target, "references", line, Some("generic_arg"));
                }
            }
        }
    }

    fn signature_refs(&mut self, source: &str, f: TsNode, line: u32) {
        if let Some(params) = f.child_by_field_name("parameters") {
            for p in named_children(params) {
                if p.kind() == "parameter" {
                    self.type_refs(source, p.child_by_field_name("type"), RefCtx::Param, line);
                }
            }
        }
        self.type_refs(
            source,
            f.child_by_field_name("return_type"),
            RefCtx::Return,
            line,
        );
    }

    // ---- items -----------------------------------------------------------------------------

    fn items(&mut self, container: TsNode) {
        for item in named_children(container) {
            if matches!(
                item.kind(),
                "mod_item"
                    | "struct_item"
                    | "enum_item"
                    | "trait_item"
                    | "function_item"
                    | "function_signature_item"
                    | "const_item"
                    | "static_item"
                    | "macro_definition"
                    | "type_item"
                    | "union_item"
            ) {
                if let Some(name) = self.field_text(item, "name") {
                    let prefix = matches!(
                        item.kind(),
                        "mod_item"
                            | "struct_item"
                            | "enum_item"
                            | "trait_item"
                            | "type_item"
                            | "union_item"
                    );
                    self.item_name(name, prefix);
                }
            }
            match item.kind() {
                "use_declaration" => {
                    self.use_decl(item);
                    self.record_uses(item);
                }
                "extern_crate_declaration" => {
                    if let Some(name) = self.field_text(item, "name") {
                        let bound = self.field_text(item, "alias").unwrap_or(name);
                        self.extern_crates
                            .push((name.to_string(), bound.to_string()));
                    }
                }
                "mod_item" => {
                    let name = self.field_text(item, "name");
                    match (item.child_by_field_name("body"), name) {
                        (Some(body), Some(name)) => {
                            if !self.inline_mods.iter().any(|m| m == name) {
                                self.inline_mods.push(name.to_string());
                            }
                            self.mod_path.push(name);
                            self.items(body);
                            self.mod_path.pop();
                        }
                        (Some(body), None) => self.items(body),
                        (None, Some(name)) => self.mods.push(RsMod {
                            inline: self.mod_path.iter().map(|s| s.to_string()).collect(),
                            name: name.to_string(),
                            path: self.path_attr(item),
                        }),
                        (None, None) => {}
                    }
                }
                "struct_item" => self.struct_item(item),
                "enum_item" => self.enum_item(item),
                "trait_item" => self.trait_item(item),
                "impl_item" => self.impl_item(item),
                "function_item" | "function_signature_item" => self.free_fn(item),
                "const_item" | "static_item" => self.value_item(item),
                "type_item" => self.type_alias(item),
                "macro_definition" => self.macro_def(item),
                "foreign_mod_item" => {
                    if let Some(body) = item.child_by_field_name("body") {
                        for f in named_children(body) {
                            if f.kind() == "function_signature_item" || f.kind() == "function_item"
                            {
                                if let Some(name) = self.field_text(f, "name") {
                                    self.item_name(name, false);
                                }
                                self.free_fn(f);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn struct_item(&mut self, n: TsNode) {
        let Some(name) = self.field_text(n, "name") else {
            return;
        };
        let id = Ctx::child_id(&self.file, name);
        let line = Ctx::line(n);
        self.def(&id, name.to_string(), line, vec![]);
        let file = self.file.clone();
        self.edge(&file, &id, "contains", line, None);
        let mark = self.push_generics(n.child_by_field_name("type_parameters"));
        if let Some(body) = n.child_by_field_name("body") {
            self.field_list_refs(&id, body);
            let outer = std::mem::replace(&mut self.self_path, vec![name]);
            self.record_fields(name, body);
            self.self_path = outer;
        }
        self.generics.truncate(mark);
    }

    /// The fields of struct `owner` whose type is a path (module resolution of `self.f.m()`).
    fn record_fields(&mut self, owner: &str, body: TsNode) {
        let mut found: Vec<(String, TsNode)> = Vec::new();
        match body.kind() {
            "field_declaration_list" => {
                for f in named_children(body) {
                    if let (Some(name), Some(ty)) =
                        (self.field_text(f, "name"), f.child_by_field_name("type"))
                    {
                        found.push((name.to_string(), ty));
                    }
                }
            }
            "ordered_field_declaration_list" => {
                let mut cursor = body.walk();
                let types: Vec<TsNode> = body.children_by_field_name("type", &mut cursor).collect();
                for (i, t) in types.into_iter().enumerate() {
                    found.push((i.to_string(), t));
                }
            }
            _ => {}
        }
        for (name, ty) in found {
            let mut segs = Vec::new();
            let Some(wrapped) = self.field_type_path(ty, false, &mut segs) else {
                continue;
            };
            if segs.first().is_some_and(|s| self.generics.contains(s)) {
                continue;
            }
            let inner = self.written_ty(ty).and_then(|t| t.inner);
            self.fields.push(RsField {
                owner: owner.to_string(),
                depth: self.mod_path.len() as u32,
                name,
                ty: segs.iter().map(|s| s.to_string()).collect(),
                wrapped,
                inner,
            });
        }
    }

    /// The path a field type names once `&`, `&mut`, `Box`, `Arc` and `Rc` are peeled, generic
    /// arguments dropped; `Some(wrapped)` when it is a plain path.
    fn field_type_path(&self, ty: TsNode, wrapped: bool, out: &mut Vec<&'a str>) -> Option<bool> {
        match ty.kind() {
            "reference_type" => self.field_type_path(ty.child_by_field_name("type")?, wrapped, out),
            "type_identifier" => {
                out.push(self.text(ty));
                Some(wrapped)
            }
            "scoped_type_identifier" => {
                if let Some(p) = ty.child_by_field_name("path") {
                    if !self.path_segments(p, out) {
                        return None;
                    }
                }
                out.push(self.field_text(ty, "name")?);
                Some(wrapped)
            }
            "generic_type" => {
                let base = ty.child_by_field_name("type")?;
                let args: Vec<TsNode> =
                    named_children(ty.child_by_field_name("type_arguments")?).collect();
                if base.kind() == "type_identifier"
                    && matches!(self.text(base), "Box" | "Arc" | "Rc")
                    && args.len() == 1
                {
                    return self.field_type_path(args[0], true, out);
                }
                self.field_type_path(base, wrapped, out)
            }
            _ => None,
        }
    }

    /// Field type references from `source` for a struct body or an enum variant body.
    fn field_list_refs(&mut self, source: &str, body: TsNode) {
        match body.kind() {
            "field_declaration_list" => {
                for f in named_children(body) {
                    if f.kind() == "field_declaration" {
                        let line = Ctx::line(f);
                        self.type_refs(source, f.child_by_field_name("type"), RefCtx::Field, line);
                    }
                }
            }
            "ordered_field_declaration_list" => {
                let mut cursor = body.walk();
                let types: Vec<TsNode> = body.children_by_field_name("type", &mut cursor).collect();
                for t in types {
                    self.type_refs(source, Some(t), RefCtx::Field, Ctx::line(t));
                }
            }
            _ => {}
        }
    }

    fn enum_item(&mut self, n: TsNode) {
        let Some(name) = self.field_text(n, "name") else {
            return;
        };
        let id = Ctx::child_id(&self.file, name);
        let line = Ctx::line(n);
        self.def(&id, name.to_string(), line, vec![]);
        let file = self.file.clone();
        self.edge(&file, &id, "contains", line, None);
        let mark = self.push_generics(n.child_by_field_name("type_parameters"));
        if let Some(list) = n.child_by_field_name("body") {
            for v in named_children(list) {
                if v.kind() != "enum_variant" {
                    continue;
                }
                let Some(vname) = self.field_text(v, "name") else {
                    continue;
                };
                let vid = Ctx::child_id(&id, vname);
                let vline = Ctx::line(v);
                if self.def(&vid, vname.to_string(), vline, vec![]) {
                    self.variant_ids.insert(vid.clone());
                }
                self.edge(&id, &vid, "case_of", vline, None);
                self.variants.entry((name, vname)).or_insert(vid);
                if let Some(body) = v.child_by_field_name("body") {
                    self.field_list_refs(&id, body);
                }
            }
        }
        self.generics.truncate(mark);
    }

    fn trait_item(&mut self, n: TsNode) {
        let Some(name) = self.field_text(n, "name") else {
            return;
        };
        let id = Ctx::child_id(&self.file, name);
        let line = Ctx::line(n);
        self.def(&id, name.to_string(), line, vec![]);
        let file = self.file.clone();
        self.edge(&file, &id, "contains", line, None);
        let mark = self.push_generics(n.child_by_field_name("type_parameters"));
        if let Some(bounds) = n.child_by_field_name("bounds") {
            for b in named_children(bounds) {
                self.bound_refs(&id, b, "inherits", line);
            }
        }
        if let Some(body) = n.child_by_field_name("body") {
            let outer = std::mem::take(&mut self.self_path);
            self.assoc_items(&id, None, body);
            self.self_path = outer;
        }
        self.generics.truncate(mark);
    }

    fn impl_item(&mut self, n: TsNode) {
        let Some(ty) = n.child_by_field_name("type") else {
            return;
        };
        let text = self.text(ty);
        let owner = Ctx::child_id(&self.file, text);
        let line = Ctx::line(n);
        let params = n.child_by_field_name("type_parameters");
        let trait_node = n.child_by_field_name("trait");
        if !self.emitted.contains(&owner) {
            let mut extra = vec![];
            if trait_node.is_none() {
                if let Some(key) = self.impl_key(n, ty, params) {
                    extra.push(("_rust_impl_key".to_string(), Attr::Str(key)));
                }
            }
            self.def(&owner, text.to_string(), line, extra);
            let mut segs = Vec::new();
            if ty.kind() != "reference_type"
                && self.field_type_path(ty, false, &mut segs) == Some(false)
            {
                self.impls.push(RsImpl {
                    owner: owner.clone(),
                    ty: segs.iter().map(|s| s.to_string()).collect(),
                    depth: self.mod_path.len() as u32,
                });
            }
        }
        let mark = self.push_generics(params);
        if let Some(t) = trait_node {
            self.bound_refs(&owner, t, "implements", line);
        }
        let base = match ty.kind() {
            "type_identifier" => Some(text),
            "generic_type" | "scoped_type_identifier" => {
                let t = if ty.kind() == "generic_type" {
                    ty.child_by_field_name("type")
                } else {
                    Some(ty)
                };
                t.and_then(|t| match t.kind() {
                    "type_identifier" => Some(self.text(t)),
                    "scoped_type_identifier" => self.field_text(t, "name"),
                    _ => None,
                })
            }
            _ => None,
        };
        let key = self
            .first_node
            .get(&owner)
            .map(|&i| {
                let node = &self.cx.facts.nodes[i];
                match node.attr("_rust_impl_key") {
                    Some(Attr::Str(k)) => k.clone(),
                    _ => node.label.clone(),
                }
            })
            .unwrap_or_else(|| text.to_string());
        let scope = ImplScope {
            owner: owner.clone(),
            self_ty: base.unwrap_or(""),
            key,
        };
        let mut path = Vec::new();
        // What `Self` names in the body: the impl's plain type path (`impl<T> Trait for T` and
        // `impl Trait for &Foo` have none).
        let plain = ty.kind() != "reference_type"
            && self.field_type_path(ty, false, &mut path) == Some(false);
        if !plain || path.first().is_some_and(|s| self.generics.contains(s)) {
            path.clear();
        }
        let outer = std::mem::replace(&mut self.self_path, path);
        if let Some(body) = n.child_by_field_name("body") {
            self.assoc_items(&owner, Some(&scope), body);
        }
        self.self_path = outer;
        self.generics.truncate(mark);
    }

    /// `Name/n` for an inherent impl over plain, unbounded type parameters (§11.1).
    fn impl_key(&self, n: TsNode, ty: TsNode, params: Option<TsNode>) -> Option<String> {
        let params = params?;
        if named_children(n).any(|c| c.kind() == "where_clause") {
            return None;
        }
        let mut names = Vec::new();
        for p in named_children(params) {
            if p.kind() != "type_parameter"
                || p.child_by_field_name("bounds").is_some()
                || p.child_by_field_name("default_type").is_some()
            {
                return None;
            }
            names.push(self.field_text(p, "name")?);
        }
        if ty.kind() != "generic_type" {
            return None;
        }
        let base = ty.child_by_field_name("type")?;
        if base.kind() != "type_identifier" {
            return None;
        }
        let args: Vec<TsNode> = named_children(ty.child_by_field_name("type_arguments")?).collect();
        if args.len() != names.len()
            || !args
                .iter()
                .all(|a| a.kind() == "type_identifier" && names.contains(&self.text(*a)))
        {
            return None;
        }
        Some(format!("{}/{}", self.text(base), names.len()))
    }

    /// Methods and associated consts of a trait (`imp` None) or an impl.
    fn assoc_items(&mut self, owner: &str, imp: Option<&ImplScope<'a>>, body: TsNode) {
        for item in named_children(body) {
            match item.kind() {
                "function_item" | "function_signature_item" => {
                    let Some(name) = self.field_text(item, "name") else {
                        continue;
                    };
                    let id = Ctx::child_id(owner, name);
                    let line = Ctx::line(item);
                    let kept = self.def(&id, format!(".{name}()"), line, vec![]);
                    if kept {
                        self.methods.insert(name, id.clone());
                    }
                    self.edge(owner, &id, "method", line, None);
                    let mark = self.push_generics(item.child_by_field_name("type_parameters"));
                    self.signature_refs(&id, item, line);
                    if kept {
                        self.record_return(&id, item);
                    }
                    self.generics.truncate(mark);
                    if let Some(block) = item.child_by_field_name("body") {
                        // Inside an impl, `Self` is the impl's type and `self.m()` prefers the
                        // impl's own methods; inside a trait neither applies.
                        if let Some(imp) = imp {
                            if !imp.self_ty.is_empty() {
                                push_distinct(
                                    self.type_methods.entry((imp.self_ty, name)).or_default(),
                                    &id,
                                );
                            }
                            self.owner_methods
                                .entry((owner.to_string(), name))
                                .or_insert_with(|| id.clone());
                        }
                        self.calls_in(&id, imp.cloned(), block, kept);
                        if kept {
                            self.infer_fn(&id, item, block, imp.is_some());
                        }
                    }
                }
                "const_item" => {
                    let Some(name) = self.field_text(item, "name") else {
                        continue;
                    };
                    let id = Ctx::child_id(owner, name);
                    let line = Ctx::line(item);
                    self.def(&id, format!(".{name}"), line, vec![]);
                    self.edge(owner, &id, "contains", line, None);
                    self.type_refs(&id, item.child_by_field_name("type"), RefCtx::Field, line);
                }
                _ => {}
            }
        }
    }

    fn free_fn(&mut self, n: TsNode) {
        let Some(name) = self.field_text(n, "name") else {
            return;
        };
        let id = Ctx::child_id(&self.file, name);
        let line = Ctx::line(n);
        // Only a function whose node the build keeps is a call target.
        let kept = self.def(&id, format!("{name}()"), line, vec![]);
        if kept {
            self.free_fns.insert(name, id.clone());
        }
        let file = self.file.clone();
        self.edge(&file, &id, "contains", line, None);
        let mark = self.push_generics(n.child_by_field_name("type_parameters"));
        self.signature_refs(&id, n, line);
        if kept {
            self.record_return(&id, n);
        }
        self.generics.truncate(mark);
        if let Some(block) = n.child_by_field_name("body") {
            self.calls_in(&id, None, block, kept);
            if kept {
                self.infer_fn(&id, n, block, false);
            }
        }
    }

    /// `type Name = a::B;` for module resolution (`Name::new()` is `a::B::new()`).
    fn type_alias(&mut self, n: TsNode) {
        let (Some(name), Some(ty)) = (self.field_text(n, "name"), n.child_by_field_name("type"))
        else {
            return;
        };
        let mark = self.push_generics(n.child_by_field_name("type_parameters"));
        let mut segs = Vec::new();
        if self.field_type_path(ty, false, &mut segs) == Some(false)
            && !segs.first().is_some_and(|s| self.generics.contains(s))
        {
            self.aliases.push(RsAlias {
                name: name.to_string(),
                depth: self.mod_path.len() as u32,
                ty: segs.iter().map(|s| s.to_string()).collect(),
            });
        }
        self.generics.truncate(mark);
    }

    fn value_item(&mut self, n: TsNode) {
        let Some(name) = self.field_text(n, "name") else {
            return;
        };
        let id = Ctx::child_id(&self.file, name);
        let line = Ctx::line(n);
        self.def(&id, name.to_string(), line, vec![]);
        let file = self.file.clone();
        self.edge(&file, &id, "contains", line, None);
        self.type_refs(&id, n.child_by_field_name("type"), RefCtx::Field, line);
    }

    fn macro_def(&mut self, n: TsNode) {
        let Some(name) = self.field_text(n, "name") else {
            return;
        };
        let id = Ctx::child_id(&self.file, &format!("macro_{name}"));
        let line = Ctx::line(n);
        self.def(&id, format!("{name}!"), line, vec![]);
        let file = self.file.clone();
        self.edge(&file, &id, "contains", line, None);
        self.macros.entry(name).or_insert(id);
    }

    // ---- use ---------------------------------------------------------------------------------

    fn use_decl(&mut self, n: TsNode) {
        let Some(arg) = n.child_by_field_name("argument") else {
            return;
        };
        let mut leaves = Vec::new();
        self.use_leaves(arg, None, &mut leaves);
        let file = self.file.clone();
        for (id, line) in leaves {
            self.edge(&file, &id, "imports_from", line, Some("import"));
            self.imports.push(id);
        }
    }

    fn item_name(&mut self, name: &'a str, prefix: bool) {
        match self.item_seen.get(name) {
            Some(&i) => self.items[i].1 |= prefix,
            None => {
                self.item_seen.insert(name, self.items.len());
                self.items.push((name.to_string(), prefix));
            }
        }
    }

    /// `#[path = "…"]` among the attributes written before an item.
    fn path_attr(&self, item: TsNode) -> Option<String> {
        let mut prev = item.prev_named_sibling();
        while let Some(p) = prev.filter(|p| matches!(p.kind(), "attribute_item" | "line_comment")) {
            let text = self.text(p);
            if let Some(rest) = text
                .strip_prefix("#[")
                .map(str::trim_start)
                .and_then(|t| t.strip_prefix("path"))
                .map(str::trim_start)
                .and_then(|t| t.strip_prefix('='))
            {
                let value = rest.trim().trim_end_matches(']').trim();
                if let Some(v) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
                    return Some(v.to_string());
                }
            }
            prev = p.prev_named_sibling();
        }
        None
    }

    /// Full-path leaves of a `use` declaration (module resolution, `crate::resolve::rust`).
    fn record_uses(&mut self, n: TsNode) {
        if let Some(arg) = n.child_by_field_name("argument") {
            let mut prefix = Vec::new();
            self.use_tree(arg, &mut prefix);
        }
    }

    /// Segments of a path (`a::b::c`); `false` for anything that is not a plain path.
    fn path_segments(&self, n: TsNode, out: &mut Vec<&'a str>) -> bool {
        match n.kind() {
            "scoped_identifier" => {
                match n.child_by_field_name("path") {
                    Some(p) => {
                        if !self.path_segments(p, out) {
                            return false;
                        }
                    }
                    None => out.push(""),
                }
                match self.field_text(n, "name") {
                    Some(name) => {
                        out.push(name);
                        true
                    }
                    None => false,
                }
            }
            "identifier" | "crate" | "self" | "super" => {
                out.push(self.text(n));
                true
            }
            _ => false,
        }
    }

    fn use_tree(&mut self, n: TsNode, prefix: &mut Vec<&'a str>) {
        let mark = prefix.len();
        match n.kind() {
            "use_list" => {
                for c in named_children(n) {
                    self.use_tree(c, prefix);
                }
            }
            "scoped_use_list" => {
                let ok = match n.child_by_field_name("path") {
                    Some(p) => self.path_segments(p, prefix),
                    None => true,
                };
                if let (true, Some(list)) = (ok, n.child_by_field_name("list")) {
                    self.use_tree(list, prefix);
                }
            }
            "use_wildcard" => {
                if named_children(n)
                    .next()
                    .is_none_or(|p| self.path_segments(p, prefix))
                {
                    self.push_use(prefix, None, true, n);
                }
            }
            "use_as_clause" => {
                let ok = n
                    .child_by_field_name("path")
                    .is_some_and(|p| self.path_segments(p, prefix));
                if ok {
                    if prefix.last() == Some(&"self") {
                        prefix.pop();
                    }
                    let alias = self.field_text(n, "alias").filter(|a| *a != "_");
                    self.push_use(prefix, alias, false, n);
                }
            }
            _ => {
                if self.path_segments(n, prefix) {
                    if prefix.last() == Some(&"self") && prefix.len() > 1 {
                        prefix.pop();
                    }
                    let bound = prefix.last().copied();
                    self.push_use(prefix, bound, false, n);
                }
            }
        }
        prefix.truncate(mark);
    }

    fn push_use(&mut self, path: &[&str], bound: Option<&str>, glob: bool, n: TsNode) {
        if path.is_empty() {
            return;
        }
        self.uses.push(RsUse {
            path: path.iter().map(|s| s.to_string()).collect(),
            bound: bound.map(str::to_string),
            glob,
            depth: self.mod_path.len() as u32,
            in_fn: self.in_body,
            func: self.in_body.then(|| self.body_fn.clone()).flatten(),
            line: Ctx::line(n),
        });
    }

    /// Last segment of a path in a `use` tree (`a::b` → `b`; `self` → the enclosing segment).
    fn path_last(&self, n: TsNode, parent: Option<&'a str>) -> Option<&'a str> {
        let seg = match n.kind() {
            "scoped_identifier" => self.field_text(n, "name")?,
            _ => self.text(n),
        };
        if seg == "self" {
            if n.kind() == "scoped_identifier" {
                return n
                    .child_by_field_name("path")
                    .and_then(|p| self.path_last(p, parent));
            }
            return parent;
        }
        Some(seg)
    }

    fn use_leaves(&self, n: TsNode, parent: Option<&'a str>, out: &mut Vec<(String, u32)>) {
        let line = Ctx::line(n);
        let mut leaf = |name: String| {
            let id = slug(&name);
            if !id.is_empty() {
                out.push((id, line));
            }
        };
        match n.kind() {
            "use_list" => {
                for c in named_children(n) {
                    self.use_leaves(c, parent, out);
                }
            }
            "scoped_use_list" => {
                let prefix = n
                    .child_by_field_name("path")
                    .and_then(|p| self.path_last(p, parent))
                    .or(parent);
                if let Some(list) = n.child_by_field_name("list") {
                    self.use_leaves(list, prefix, out);
                }
            }
            "use_as_clause" => {
                let name = n
                    .child_by_field_name("path")
                    .and_then(|p| self.path_last(p, parent));
                if let (Some(name), Some(alias)) = (name, self.field_text(n, "alias")) {
                    leaf(format!("{name} as {alias}"));
                }
            }
            "use_wildcard" => {
                let name = named_children(n)
                    .next()
                    .and_then(|p| self.path_last(p, parent))
                    .or(parent);
                if let Some(name) = name {
                    leaf(name.to_string());
                }
            }
            "identifier" | "scoped_identifier" | "crate" | "self" | "super" => {
                if let Some(name) = self.path_last(n, parent) {
                    leaf(name.to_string());
                }
            }
            _ => {}
        }
    }

    // ---- calls ---------------------------------------------------------------------------------

    /// Record the call sites of one function body (nested items are skipped; closures and async
    /// blocks belong to the enclosing function; macro arguments are opaque).
    fn calls_in(&mut self, caller: &str, scope: Option<ImplScope<'a>>, body: TsNode, kept: bool) {
        let caller_idx = self.callers.len();
        self.caller_kept.push(kept);
        let self_ty = scope.as_ref().map(|s| s.self_ty).filter(|t| !t.is_empty());
        self.callers.push((caller.to_string(), scope));
        let mut stack = vec![body];
        while let Some(n) = stack.pop() {
            match n.kind() {
                // A `use` in a body brings no edge Graphify knows, but it is a dependency of the
                // file (module resolution).
                "use_declaration" => {
                    self.in_body = true;
                    self.body_fn = Some(caller.to_string());
                    self.record_uses(n);
                    self.in_body = false;
                    self.body_fn = None;
                    continue;
                }
                "function_item" | "impl_item" | "trait_item" | "mod_item" | "macro_definition"
                | "foreign_mod_item" | "struct_item" | "enum_item" | "union_item" => continue,
                "macro_invocation" => {
                    if let Some(m) = n.child_by_field_name("macro") {
                        if m.kind() == "identifier" {
                            self.sites.push(Site {
                                caller: caller_idx,
                                callee: Callee::Macro(self.text(m)),
                                line: Ctx::line(n),
                                qual: Vec::new(),
                                fields: Vec::new(),
                                depth: self.mod_path.len() as u32,
                            });
                        }
                    }
                    continue;
                }
                "call_expression" => {
                    if let Some(f) = n.child_by_field_name("function") {
                        if let Some(callee) = self.callee(f, self_ty) {
                            let mut qual = Vec::new();
                            if f.kind() == "scoped_identifier" && self.path_segments(f, &mut qual) {
                                qual.pop();
                            } else {
                                qual.clear();
                            }
                            let fields = self.receiver_fields(f);
                            self.sites.push(Site {
                                caller: caller_idx,
                                callee,
                                line: Ctx::line(n),
                                qual,
                                fields,
                                depth: self.mod_path.len() as u32,
                            });
                        }
                    }
                }
                _ => {}
            }
            let count = n.named_child_count();
            for i in (0..count).rev() {
                if let Some(c) = n.named_child(i) {
                    stack.push(c);
                }
            }
        }
    }

    /// `[a, b]` for a `self.a.b.m` callee; empty for any other receiver.
    fn receiver_fields(&self, f: TsNode) -> Vec<&'a str> {
        let mut out = Vec::new();
        if f.kind() != "field_expression" {
            return out;
        }
        let mut cur = f.child_by_field_name("value");
        while let Some(n) = cur {
            match n.kind() {
                "self" => {
                    out.reverse();
                    return out;
                }
                "field_expression" => {
                    let Some(field) = n.child_by_field_name("field") else {
                        break;
                    };
                    if !matches!(field.kind(), "field_identifier" | "integer_literal") {
                        break;
                    }
                    out.push(self.text(field));
                    cur = n.child_by_field_name("value");
                }
                _ => break,
            }
        }
        Vec::new()
    }

    fn callee(&self, f: TsNode, self_ty: Option<&'a str>) -> Option<Callee<'a>> {
        match f.kind() {
            "identifier" => Some(Callee::Bare(self.text(f))),
            "field_expression" => {
                let field = f.child_by_field_name("field")?;
                let on_self = f
                    .child_by_field_name("value")
                    .is_some_and(|v| v.kind() == "self");
                (field.kind() == "field_identifier")
                    .then(|| Callee::Method(self.text(field), on_self))
            }
            "scoped_identifier" => {
                let name = self.field_text(f, "name")?;
                let Some(path) = f.child_by_field_name("path") else {
                    return Some(Callee::Module(name));
                };
                let qual = match path.kind() {
                    "identifier" => self.text(path),
                    "scoped_identifier" => self.field_text(path, "name")?,
                    "crate" | "self" | "super" => return Some(Callee::Module(name)),
                    _ => return None,
                };
                if qual == "Self" {
                    self_ty.map(|t| Callee::Typed(t, name))
                } else if starts_upper(qual) {
                    Some(Callee::Typed(qual, name))
                } else {
                    Some(Callee::Module(name))
                }
            }
            _ => None,
        }
    }

    /// Resolve the recorded call sites against this file (§11.5), then hand the rest to the
    /// cross-file pass.
    fn resolve_calls(&mut self) -> Pending {
        // In-file constructor targets: every non-function node by label, last wins.
        let mut ctor: HashMap<String, String> = HashMap::new();
        for n in &self.cx.facts.nodes {
            if n.kind != crate::facts::NodeKind::File
                && !n.label.starts_with('.')
                && !n.label.ends_with("()")
                && !n.label.ends_with('!')
            {
                ctor.insert(n.label.clone(), n.id.clone());
            }
        }
        let unique = |v: Option<&Vec<String>>| -> Option<String> {
            match v {
                Some(v) if v.len() == 1 => Some(v[0].clone()),
                _ => None,
            }
        };
        let mut pending = Vec::new();
        let mut self_calls = Vec::new();
        let mut path_calls = Vec::new();
        let mut field_calls = Vec::new();
        // One edge per (caller id, callee), at the first call: `#[cfg]` twins share a caller id.
        let mut seen: HashSet<(String, String)> = HashSet::new();
        let mut seen_paths: HashSet<(usize, &[&str], &[&str], &str)> = HashSet::new();
        let sites = std::mem::take(&mut self.sites);
        for site in &sites {
            let (caller, owner) = &self.callers[site.caller];
            let (target, relation) = match site.callee {
                Callee::Bare(name) | Callee::Module(name) => {
                    let bare = matches!(site.callee, Callee::Bare(_));
                    // Q15: a bare name may be a local binding (`filter(entry)`), so Graphify's
                    // builtin denylist stays for it; a path-qualified call cannot be one.
                    let free = if bare && call_denied(name) {
                        None
                    } else {
                        self.free_fns.get(name)
                    };
                    if let Some(id) = free {
                        (Some(id.clone()), "calls")
                    } else if let Some(id) = ctor.get(name).filter(|_| !ctor_denied(name)) {
                        (Some(id.clone()), "references")
                    } else {
                        if bare && !call_denied(name) {
                            pending.push(PendingCall {
                                caller: caller.clone(),
                                name: name.to_string(),
                                line: site.line,
                                common: cross_file_denied(name),
                                kept: self.caller_kept[site.caller],
                            });
                        }
                        (None, "")
                    }
                }
                Callee::Typed(ty, name) => {
                    if let Some(id) = unique(self.type_methods.get(&(ty, name))) {
                        // The id may be held by a same-slug variant (`Sync` + `fn sync`).
                        let rel = if self.variant_ids.contains(&id) {
                            "references"
                        } else {
                            "calls"
                        };
                        (Some(id), rel)
                    } else if self.enums.contains(ty) {
                        // The named enum's own variant. Graphify instead takes the last node of
                        // the file labelled `name` and skips denylisted names (`File`, `Number`).
                        (self.variants.get(&(ty, name)).cloned(), "references")
                    } else {
                        // Enum declared elsewhere: the file's node labelled `name` (often the
                        // stub of the type the variant wraps), as Graphify does.
                        let id = ctor.get(name).filter(|_| !ctor_denied(name)).cloned();
                        (id, "references")
                    }
                }
                Callee::Method(name, on_self) => {
                    if call_denied(name) {
                        (None, "")
                    } else {
                        let imp = owner.as_ref().filter(|_| on_self);
                        let own =
                            imp.and_then(|s| self.owner_methods.get(&(s.owner.clone(), name)));
                        let found = own.or_else(|| self.methods.get(name)).cloned();
                        if let (None, Some(s)) = (&found, imp) {
                            if !cross_file_denied(name) {
                                self_calls.push(SelfCall {
                                    caller: caller.clone(),
                                    owner_key: s.key.clone(),
                                    name: name.to_string(),
                                    line: site.line,
                                });
                            }
                        }
                        (found, "calls")
                    }
                }
                Callee::Macro(name) => (self.macros.get(name).cloned(), "calls"),
            };
            let self_ty = owner.as_ref().map_or("", |s| s.self_ty);
            let name = match site.callee {
                Callee::Bare(n) | Callee::Module(n) | Callee::Typed(_, n) => n,
                Callee::Method(n, _) | Callee::Macro(n) => n,
            };
            let kept = self.caller_kept[site.caller];
            if kept
                && !site.qual.is_empty()
                && seen_paths.insert((site.caller, &site.qual, &[], name))
            {
                let qual: Vec<String> = if site.qual == ["Self"] {
                    vec![self_ty.to_string()]
                } else {
                    site.qual.iter().map(|s| s.to_string()).collect()
                };
                if qual.iter().skip(1).all(|s| !s.is_empty()) && qual != [""] {
                    path_calls.push(PathCall {
                        caller: caller.clone(),
                        qual,
                        name: name.to_string(),
                        depth: site.depth,
                        line: site.line,
                        in_file: target.clone(),
                    });
                }
            }
            if kept
                && !site.fields.is_empty()
                && !self_ty.is_empty()
                && seen_paths.insert((site.caller, &[], &site.fields, name))
            {
                field_calls.push(FieldCall {
                    caller: caller.clone(),
                    self_ty: self_ty.to_string(),
                    depth: site.depth,
                    fields: site.fields.iter().map(|s| s.to_string()).collect(),
                    name: name.to_string(),
                    line: site.line,
                    in_file: target.clone(),
                });
            }
            let Some(target) = target else { continue };
            if target == *caller || !seen.insert((caller.clone(), target.clone())) {
                continue;
            }
            let context = if relation == "calls" {
                "call"
            } else {
                "constructor"
            };
            let caller = caller.clone();
            self.edge(&caller, &target, relation, site.line, Some(context));
        }
        Pending {
            calls: pending,
            self_calls,
            path_calls,
            field_calls,
        }
    }

    fn finish(mut self) {
        let Pending {
            calls,
            self_calls,
            path_calls,
            field_calls,
        } = self.resolve_calls();
        let (tys, method_calls) = self.finish_inference();
        // `_rust_declaration_count` lands on whichever node holds the declared id first.
        for (id, count) in std::mem::take(&mut self.decl_count) {
            if let Some(&i) = self.first_node.get(&id) {
                self.cx.facts.nodes[i]
                    .attrs
                    .insert(0, ("_rust_declaration_count".into(), Attr::Int(count)));
            }
        }
        let mut externals = Vec::new();
        let mut seen = HashSet::new();
        for id in std::mem::take(&mut self.imports) {
            if !self.stubs.contains(&id) && seen.insert(id.clone()) {
                externals.push(id);
            }
        }
        self.cx.facts.lang = LangFacts::Rust(RustFacts {
            externals,
            calls,
            self_calls,
            uses: std::mem::take(&mut self.uses),
            mods: std::mem::take(&mut self.mods),
            items: std::mem::take(&mut self.items),
            extern_crates: std::mem::take(&mut self.extern_crates),
            path_calls,
            field_calls,
            fields: std::mem::take(&mut self.fields),
            impls: std::mem::take(&mut self.impls),
            traits: std::mem::take(&mut self.traits),
            inline_mods: std::mem::take(&mut self.inline_mods),
            aliases: std::mem::take(&mut self.aliases),
            tys,
            method_calls,
            returns: std::mem::take(&mut self.returns),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::extract_file;
    use crate::facts::{FileFacts, Loc};
    use crate::lang::Lang;

    fn facts(src: &str) -> FileFacts {
        extract_file(Lang::Rust, "src/a.rs", src.as_bytes())
    }

    fn rust(f: &FileFacts) -> &RustFacts {
        match &f.lang {
            LangFacts::Rust(r) => r,
            other => panic!("not Rust facts: {other:?}"),
        }
    }

    /// `source -relation-> target [context] Lline` for every edge, in emission order.
    fn edges(f: &FileFacts) -> Vec<String> {
        f.edges
            .iter()
            .map(|e| {
                let line = match e.loc {
                    Loc::Line(n) => n,
                    _ => 0,
                };
                let ctx = e.context.as_deref().unwrap_or("-");
                format!(
                    "{} -{}-> {} [{ctx}] L{line}",
                    e.source, e.relation, e.target
                )
            })
            .collect()
    }

    fn has(f: &FileFacts, edge: &str) -> bool {
        edges(f).iter().any(|e| e == edge)
    }

    #[test]
    fn use_leaves_follow_the_last_segment() {
        let f = facts(
            "use std::{ffi::OsStr,\n io::{self, Read}};\nuse a::b::{self as bee};\n\
             use x::y::Z as _;\nuse super::model::*;\nuse q::{r::*, s::T as Tee};\n\
             fn f() { use inner::Hidden; }\n",
        );
        let imports: Vec<String> = edges(&f)
            .into_iter()
            .filter(|e| e.contains("imports_from"))
            .collect();
        assert_eq!(
            imports,
            [
                "src_a -imports_from-> osstr [import] L1",
                "src_a -imports_from-> io [import] L2",
                "src_a -imports_from-> read [import] L2",
                "src_a -imports_from-> b_as_bee [import] L3",
                "src_a -imports_from-> z_as [import] L4",
                "src_a -imports_from-> model [import] L5",
                "src_a -imports_from-> r [import] L6",
                "src_a -imports_from-> t_as_tee [import] L6",
            ]
        );
        assert_eq!(rust(&f).externals.len(), 8);
    }

    #[test]
    fn module_facts_record_full_use_paths_mods_and_items() {
        let f = facts(
            "use std::{io::{self, Read}, fmt as f};\nuse ::abs::X;\nuse super::m::*;\n\
             use a::B as _;\n#[path = \"other.rs\"]\nmod renamed;\nmod plain;\n\
             extern crate grep_cli as cli;\nmod inner {\n    use super::*;\n    mod deep;\n    \
             pub struct S;\n}\nfn regex() { use crate::x::Y; }\ntype Alias = u8;\n",
        );
        let r = rust(&f);
        let uses: Vec<(String, Option<&str>, bool, u32)> = r
            .uses
            .iter()
            .map(|u| (u.path.join("::"), u.bound.as_deref(), u.glob, u.depth))
            .collect();
        assert_eq!(
            uses,
            [
                ("std::io".to_string(), Some("io"), false, 0),
                ("std::io::Read".to_string(), Some("Read"), false, 0),
                ("std::fmt".to_string(), Some("f"), false, 0),
                ("::abs::X".to_string(), Some("X"), false, 0),
                ("super::m".to_string(), None, true, 0),
                ("a::B".to_string(), None, false, 0),
                ("super".to_string(), None, true, 1),
                ("crate::x::Y".to_string(), Some("Y"), false, 0),
            ]
        );
        let mods: Vec<(String, Option<&str>)> = r
            .mods
            .iter()
            .map(|m| {
                let mut p = m.inline.clone();
                p.push(m.name.clone());
                (p.join("::"), m.path.as_deref())
            })
            .collect();
        assert_eq!(
            mods,
            [
                ("renamed".to_string(), Some("other.rs")),
                ("plain".to_string(), None),
                ("inner::deep".to_string(), None),
            ]
        );
        assert_eq!(r.extern_crates, [("grep_cli".into(), "cli".into())]);
        let items: Vec<(&str, bool)> = r.items.iter().map(|(n, p)| (n.as_str(), *p)).collect();
        assert_eq!(
            items,
            [
                ("renamed", true),
                ("plain", true),
                ("inner", true),
                ("deep", true),
                ("S", true),
                ("regex", false),
                ("Alias", true),
            ]
        );
    }

    #[test]
    fn an_import_used_as_a_type_becomes_the_stub() {
        let f = facts("use std::io::Read;\nfn a(c: &dyn Read) {}\n");
        assert!(rust(&f).externals.is_empty());
        let stub = f.nodes.iter().find(|n| n.id == "read").unwrap();
        assert_eq!(stub.label, "Read");
        assert!(has(&f, "src_a -imports_from-> read [import] L1"));
    }

    #[test]
    fn type_references_carry_their_context() {
        let f = facts(
            "pub struct Error;\n\
             fn g<W: Write>(w: W, a: &[Box<dyn Shape>], b: (Tu, fmt::Formatter<'_>)) \
             -> Result<Out, Error> { todo!() }\n\
             struct S { inner: Vec<Item>, me: other::S }\n",
        );
        let refs: Vec<String> = edges(&f)
            .into_iter()
            .filter(|e| e.contains("references"))
            .collect();
        assert_eq!(
            refs,
            [
                // `W` is a generic parameter (Q14): no edge, no stub.
                "src_a_g -references-> shape [generic_arg] L2",
                "src_a_g -references-> tu [parameter_type] L2",
                "src_a_g -references-> formatter [parameter_type] L2",
                "src_a_g -references-> out [generic_arg] L2",
                // `Error` is denylisted, but this file declares it.
                "src_a_g -references-> src_a_error [generic_arg] L2",
                // `other::S` inside `S` would be a self-loop and is dropped.
                "src_a_s -references-> item [generic_arg] L3",
            ]
        );
        assert!(!f.nodes.iter().any(|n| n.id == "w"));
    }

    #[test]
    fn impl_owners_and_trait_bounds() {
        let f = facts(
            "impl Later { fn go(&self) {} }\nstruct Later;\n\
             impl<T> Pair<T> { fn first(&self) {} }\n\
             impl<T: Clone> B<T> {}\n\
             impl Into<Other> for Later {}\n\
             trait Named: Shape + Sized + Bb<Cc> {}\n",
        );
        let later = f.nodes.iter().find(|n| n.id == "src_a_later").unwrap();
        assert_eq!(later.loc, Loc::Line(1));
        assert_eq!(later.attr("_rust_declaration_count"), Some(&Attr::Int(1)));
        let pair = f.nodes.iter().find(|n| n.id == "src_a_pair_t").unwrap();
        assert_eq!(pair.label, "Pair<T>");
        assert_eq!(
            pair.attr("_rust_impl_key"),
            Some(&Attr::Str("Pair/1".into()))
        );
        let b = f.nodes.iter().find(|n| n.id == "src_a_b_t").unwrap();
        assert_eq!(b.attr("_rust_impl_key"), None);
        // The first non-denylisted name of a trait is the implemented or inherited one.
        assert!(has(&f, "src_a_later -implements-> other [-] L5"));
        assert!(has(&f, "src_a_named -inherits-> shape [-] L6"));
        assert!(has(&f, "src_a_named -inherits-> bb [-] L6"));
        assert!(has(&f, "src_a_named -references-> cc [generic_arg] L6"));
        // Created owners hang off nothing.
        assert!(!edges(&f)
            .iter()
            .any(|e| e.ends_with("-contains-> src_a_pair_t [-] L3")));
    }

    #[test]
    fn in_file_calls() {
        let f = facts(
            "struct A;\nstruct B;\n\
             impl A { fn go(&self) {} fn own(&self) { self.go(); other.go(); } }\n\
             impl B { fn go(&self) {} fn new() -> B { B } }\n\
             enum E { Lit(u8) }\nenum F { Lit(u8) }\n\
             macro_rules! m { () => {} }\n\
             mod convert { pub fn str() {} }\n\
             fn str() {}\n\
             fn c() { E::Lit(1); Lit(2); B::new(); m!(); convert::str(); str(); helper(); c(); }\n\
             #[cfg(unix)]\nfn twin() { c(); }\n#[cfg(not(unix))]\nfn twin() { c(); }\n",
        );
        // `self.go()` prefers its own impl; any other receiver takes the file's last `.go()`.
        assert!(has(&f, "src_a_a_own -calls-> src_a_a_go [call] L3"));
        assert!(has(&f, "src_a_a_own -calls-> src_a_b_go [call] L3"));
        // A qualified variant resolves to the named enum; a bare one to the last `Lit`.
        assert!(has(
            &f,
            "src_a_c -references-> src_a_e_lit [constructor] L10"
        ));
        assert!(has(
            &f,
            "src_a_c -references-> src_a_f_lit [constructor] L10"
        ));
        assert!(has(&f, "src_a_c -calls-> src_a_b_new [call] L10"));
        assert!(has(&f, "src_a_c -calls-> src_a_macro_m [call] L10"));
        // Q15: a path-qualified builtin name resolves (`mod convert` is flattened, so both
        // `str` functions are one node); the bare `str()` adds nothing.
        assert!(has(&f, "src_a_c -calls-> src_a_str [call] L10"));
        // Recursion is not an edge; `#[cfg]` twins share one edge at the first call.
        assert!(!edges(&f)
            .iter()
            .any(|e| e.starts_with("src_a_c -calls-> src_a_c ")));
        assert!(has(&f, "src_a_twin -calls-> src_a_c [call] L12"));
        assert!(!has(&f, "src_a_twin -calls-> src_a_c [call] L14"));
        let pending: Vec<&str> = rust(&f).calls.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(pending, ["helper"]);
    }

    #[test]
    fn a_bare_builtin_name_never_resolves() {
        let f = facts("fn filter() {}\nfn c(filter: impl Fn()) { filter(); }\n");
        assert!(!edges(&f).iter().any(|e| e.contains("-calls->")));
        assert!(rust(&f).calls.is_empty());
    }

    #[test]
    fn self_calls_without_a_local_method_wait_for_the_cross_file_pass() {
        let f = facts("impl Atlas { fn rank(&self) { self.files(); self.len(); } }\n");
        let calls = &rust(&f).self_calls;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].owner_key, "Atlas");
        assert_eq!(calls[0].name, "files");
    }
}
