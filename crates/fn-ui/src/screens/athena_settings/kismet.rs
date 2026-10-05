//! A small Kismet interpreter for the Settings widget Blueprints.
//!
//! The Settings tabs describe their rows in Blueprint functions (`TabVideoOptions::Initialize Data`: `ctx(V:Quality . vcall Initialize Row Selector(label,
//! options, hover texts, ...))`) and drive row visibility / enabled state from `Construct` / `Update Data`. This runs such a function (or an event stub that
//! jumps into the `ExecuteUbergraph_*` function) over the cooked bytecode and reports every call made on a widget (`ctx(V:Widget . vcall F(..))`) or on
//! `self`, with arguments evaluated to constants.
//!
//! What it evaluates: locals, constants, `MakeLiteralText`, `SetArray`, `Switch` selects, the boolean / integer math of the Settings graphs, jumps,
//! `pushflow` / `popflow`, and zero-argument calls of functions that exist in the same package. Native functions (the ones the package does not define:
//! `IsAthena`, `GetPossibleFPS`, ...) are answered by the host through [`Natives`]; a native it cannot answer evaluates to `Val::Unknown` and branches on an
//! unknown value fall through (reported in `Run::notes`).
use crate::screens::athena_lobby::store::Store;
use fn_uasset::kismet::{Expr, Statement, TextLit};
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    /// `FText` of the bytecode: (source string, key, namespace); `key` empty = literal text
    Text { src: String, key: String, ns: String },
    Array(Vec<Val>),
    Bool(bool),
    Int(i64),
    Float(f32),
    Name(String),
    Str(String),
    /// class / object constant (`obj IconTabButton_C`)
    Obj(String),
    /// an instance variable (a widget or a Blueprint variable of the class)
    Var(String),
    Local(String),
    NoObject,
    /// an object instance given to an event (class chain, most derived first, class names without package path)
    Inst(Vec<String>),
    /// a widget created by `WidgetBlueprintLibrary::Create` (class name)
    Created(String),
    /// a struct built with `smember[Prop](L:Struct) = value` assignments (field, value)
    Struct(Vec<(String, Val)>),
    Unknown(String),
}
impl Val {
    pub fn as_array(&self) -> &[Val] { match self { Val::Array(a) => a, _ => &[] } }
    pub fn as_bool(&self) -> Option<bool> { match self { Val::Bool(b) => Some(*b), Val::Int(i) => Some(*i != 0), _ => None } }
    pub fn as_int(&self) -> Option<i64> { match self { Val::Int(i) => Some(*i), Val::Bool(b) => Some(*b as i64), _ => None } }
    pub fn as_float(&self) -> Option<f32> { match self { Val::Float(f) => Some(*f), Val::Int(i) => Some(*i as f32), _ => None } }
    pub fn as_name(&self) -> Option<&str> { match self { Val::Name(s) | Val::Obj(s) | Val::Var(s) => Some(s), _ => None } }
    pub fn is_unknown(&self) -> bool { matches!(self, Val::Unknown(_) | Val::Local(_)) }
}

#[derive(Clone, Debug)]
pub struct Call {
    /// widget / variable the call is made on (`None` = a call on `self`)
    pub on: Option<String>,
    pub func: String,
    pub args: Vec<Val>,
    /// byte offset of the statement
    pub offset: usize,
}

/// Host answers for native functions the Blueprint package does not define (`IsAthena()`, `GetPossibleFPS()`, ...). `None` = cannot answer.
pub trait Natives {
    fn call(&self, func: &str, args: &[Val]) -> Option<Val>;
    /// A query on a widget variable of the running Blueprint (`ctx(V:Message . vcall IsVisible())`); `None` = unknown.
    fn widget(&self, _on: &str, _func: &str, _args: &[Val]) -> Option<Val> { None }
}
/// No natives: every native call is unknown.
pub struct NoNatives;
impl Natives for NoNatives { fn call(&self, _: &str, _: &[Val]) -> Option<Val> { None } }

#[derive(Default)]
pub struct Run { pub calls: Vec<Call>, pub notes: Vec<String> }

fn text_of(e: &Expr) -> Option<String> { match e { Expr::StringConst(s) | Expr::UnicodeStringConst(s) => Some(s.clone()), _ => None } }

pub struct Vm<'a> {
    store: &'a Store,
    long: String,
    natives: &'a dyn Natives,
    /// the widget variable table values the host wants the graph to read (`V:Name`): reads of an instance variable return `Val::Var(name)` unless set here
    pub vars: HashMap<String, Val>,
    locals: HashMap<String, Val>,
    /// the Ubergraph persistent frame (`letpf[Prop] = value` writes, read back as `L:Prop` by the Ubergraph)
    frame: HashMap<String, Val>,
    pub run: Run,
    depth: usize,
}

impl<'a> Vm<'a> {
    pub fn new(store: &'a Store, long: &str, natives: &'a dyn Natives) -> Self {
        Vm { store, long: long.split('.').next().unwrap_or(long).to_owned(), natives, vars: HashMap::new(), locals: HashMap::new(), frame: HashMap::new(), run: Run::default(), depth: 0 }
    }

    fn val(&mut self, e: &Expr) -> Val {
        match e {
            Expr::TextConst(t) => match t {
                TextLit::Empty => Val::Text { src: String::new(), key: String::new(), ns: String::new() },
                TextLit::Localized(src, key, ns) => Val::Text { src: text_of(src).unwrap_or_default(), key: text_of(key).unwrap_or_default(), ns: text_of(ns).unwrap_or_default() },
                TextLit::Literal(s) | TextLit::Invariant(s) => Val::Text { src: text_of(s).unwrap_or_default(), key: String::new(), ns: String::new() },
                TextLit::StringTable { .. } => Val::Unknown("string table text".into()),
            },
            Expr::LocalVariable(r) | Expr::LocalOutVariable(r) => self.locals.get(&r.name).or_else(|| self.frame.get(&r.name)).cloned().unwrap_or_else(|| Val::Local(r.name.clone())),
            Expr::InstanceVariable(r) | Expr::DefaultVariable(r) => self.vars.get(&r.name).cloned().unwrap_or_else(|| Val::Var(r.name.clone())),
            Expr::IntConst(i) => Val::Int(*i as i64),
            Expr::IntConstByte(b) | Expr::ByteConst(b) => Val::Int(*b as i64),
            Expr::IntZero => Val::Int(0),
            Expr::IntOne => Val::Int(1),
            Expr::True => Val::Bool(true),
            Expr::False => Val::Bool(false),
            Expr::FloatConst(f) => Val::Float(*f),
            Expr::NameConst(n) => Val::Name(n.clone()),
            Expr::StringConst(s) | Expr::UnicodeStringConst(s) => Val::Str(s.clone()),
            Expr::ObjectConst(r) => Val::Obj(r.name.clone()),
            Expr::NoObject => Val::NoObject,
            Expr::ArrayConst { elements, .. } => Val::Array(elements.iter().map(|x| self.val(x)).collect()),
            // `cast71` = `CST_ObjectToBool`: a successful `DynamicCast` result (an instance) is true, `noobj` is false
            Expr::PrimitiveCast { kind: 71, expr } => match self.val(expr) { Val::Inst(_) | Val::Obj(_) => Val::Bool(true), Val::NoObject => Val::Bool(false), v => v },
            Expr::PrimitiveCast { expr, .. } => self.val(expr),
            Expr::DynamicCast { class, expr } => match self.val(expr) { Val::Inst(chain) => if chain.iter().any(|c| *c == class.name) { Val::Inst(chain) } else { Val::NoObject }, v => v },
            Expr::Switch { index, cases, default } => {
                let i = self.val(index);
                for (k, r) in cases { let kv = self.val(k); if same(&i, &kv) { return self.val(r); } }
                self.val(default)
            }
            Expr::CallMath { func, args } => {
                let a: Vec<Val> = args.iter().map(|x| self.val(x)).collect();
                if func.name == "BreakVector2D" && args.len() == 3 {
                    // `BreakVector2D(In, out X, out Y)`
                    if let Val::Array(v) = &a[0] { if v.len() == 2 { let (x, y) = (v[0].clone(), v[1].clone()); self.assign(&args[1], x); self.assign(&args[2], y); } }
                    return Val::NoObject;
                }
                math(&func.name, &a)
            }
            Expr::Context { object, context, .. } | Expr::ContextFailSilent { object, context, .. } => self.context_value(object, context),
            Expr::FinalFunction { func, args } => { let a: Vec<Val> = args.iter().map(|x| self.val(x)).collect(); self.native_or_user(&func.name, a) }
            Expr::VirtualFunction { name, args } | Expr::LocalVirtualFunction { name, args } => { let a: Vec<Val> = args.iter().map(|x| self.val(x)).collect(); self.native_or_user(name, a) }
            Expr::LocalFinalFunction { func, args } => { let a: Vec<Val> = args.iter().map(|x| self.val(x)).collect(); self.native_or_user(&func.name, a) }
            other => Val::Unknown(other.render()),
        }
    }

    /// Value of `ctx(object . ... : call F(..))`: a library call on a `Default__` object evaluates like a native; a call on a widget is recorded.
    fn context_value(&mut self, object: &Expr, context: &Expr) -> Val {
        let on = match object { Expr::InstanceVariable(r) | Expr::LocalVariable(r) => Some(r.name.clone()), Expr::ObjectConst(r) => Some(r.name.clone()), _ => None };
        let (name, args) = match context {
            Expr::VirtualFunction { name, args } | Expr::LocalVirtualFunction { name, args } => (name.clone(), args),
            Expr::FinalFunction { func, args } | Expr::LocalFinalFunction { func, args } => (func.name.clone(), args),
            Expr::InstanceVariable(r) => return self.vars.get(&r.name).cloned().unwrap_or_else(|| Val::Var(r.name.clone())),
            other => return Val::Unknown(other.render()),
        };
        let a: Vec<Val> = args.iter().map(|x| self.val(x)).collect();
        if on.as_deref().is_some_and(|o| o.starts_with("Default__")) {
            return match name.as_str() {
                "MakeLiteralText" => a.into_iter().next().unwrap_or(Val::Unknown("MakeLiteralText()".into())),
                "Array_Length" => Val::Int(a.first().map_or(0, |v| v.as_array().len() as i64)),
                "EqualEqual_StrStr" => match (a.first(), a.get(1)) { (Some(Val::Str(x)), Some(Val::Str(y))) => Val::Bool(x == y), _ => Val::Unknown("EqualEqual_StrStr(?)".into()) },
                // `Create(WorldContext, WidgetClass, OwningPlayer)`
                "Create" => match a.get(1) { Some(Val::Obj(c)) => Val::Created(c.clone()), _ => Val::Unknown("Create(?)".into()) },
                _ => self.natives.call(&name, &a).unwrap_or_else(|| Val::Unknown(format!("{}.{name}", on.unwrap_or_default()))),
            };
        }
        let offset = 0;
        self.run.calls.push(Call { on: on.clone(), func: name.clone(), args: a.clone(), offset });
        let r = on.as_deref().and_then(|o| self.natives.widget(o, &name, &a)).or_else(|| self.natives.call(&name, &a));
        r.unwrap_or(Val::Unknown(format!("{}.{name}", on.unwrap_or_default())))
    }

    /// A call on `self`: a function of this package with no arguments (or an `ExecuteUbergraph` stub) is executed, anything else is native.
    fn native_or_user(&mut self, name: &str, args: Vec<Val>) -> Val {
        if name.starts_with("ExecuteUbergraph") {
            if let Some(Val::Int(entry)) = args.first() { self.exec_function(name, Some(*entry as usize)); }
            return Val::Unknown(name.to_owned());
        }
        if self.depth < 8 && self.store.function(&self.long, name).is_some() {
            let params = self.store.function_params(&self.long, name);
            if params.len() == args.len() {
                self.run.calls.push(Call { on: None, func: name.to_owned(), args: args.clone(), offset: 0 });
                let p: Vec<(String, Val)> = params.into_iter().zip(args).collect();
                self.exec_with(name, None, p);
                return Val::Unknown(name.to_owned());
            }
        }
        self.run.calls.push(Call { on: None, func: name.to_owned(), args: args.clone(), offset: 0 });
        self.natives.call(name, &args).unwrap_or_else(|| Val::Unknown(format!("call {name}")))
    }

    /// Run function `name` from its first statement, or (Ubergraph) from the statement at byte offset `entry`.
    pub fn exec_function(&mut self, name: &str, entry: Option<usize>) { self.exec_with(name, entry, vec![]); }

    /// Run a function / event with its input parameters bound by name (`IsDesignTime`, `Reset Quality Selector`, ...).
    pub fn exec_with(&mut self, name: &str, entry: Option<usize>, params: Vec<(String, Val)>) {
        let Some(st) = self.store.function(&self.long, name) else { self.run.notes.push(format!("function {name} not found")); return };
        self.depth += 1;
        let saved = std::mem::replace(&mut self.locals, params.into_iter().collect());
        self.exec(&st, entry);
        self.locals = saved;
        self.depth -= 1;
    }

    fn exec(&mut self, st: &Rc<Vec<Statement>>, entry: Option<usize>) {
        // Jump targets, `pushflow` targets and the Ubergraph entry points are offsets in the LOADED script, where every object pointer operand takes 8
        // bytes while the cooked file stores a 4 byte package index (UE 4.16 `FKismetBytecodeDisassembler` / `UFunction::Serialize` fix-up): the memory
        // offset of a statement is its file offset plus 4 per pointer operand of the statements before it.
        let mut by_off: HashMap<usize, usize> = HashMap::new();
        let mut extra = 0usize;
        for (i, s) in st.iter().enumerate() { by_off.insert(s.offset + extra, i); extra += 4 * ptr_count(&s.expr); }
        let mut pc = match entry { Some(o) => match by_off.get(&o) { Some(&i) => i, None => { let mut ks: Vec<usize> = by_off.keys().copied().filter(|k| k.abs_diff(o) < 40).collect(); ks.sort(); self.run.notes.push(format!("entry {o} is not a statement (near {ks:?}; {} statements)", st.len())); return } }, None => 0 };
        let mut flow: Vec<usize> = vec![];
        let mut steps = 0usize;
        while pc < st.len() {
            steps += 1;
            if steps > 20000 { self.run.notes.push("step limit".into()); return; }
            let s = &st[pc];
            pc += 1;
            let jump = |t: usize, notes: &mut Vec<String>| -> Option<usize> { match by_off.get(&t) { Some(&i) => Some(i), None => { notes.push(format!("jump to {t:#x}: no statement")); None } } };
            match &s.expr {
                Expr::EndOfScript | Expr::Return(_) => return,
                Expr::Jump(t) => match jump(*t as usize, &mut self.run.notes) { Some(i) => pc = i, None => return },
                Expr::JumpIfNot { target, cond } => {
                    let c = self.val(cond);
                    let taken = match c.as_bool() { Some(b) => !b, None => { self.run.notes.push(format!("{:#x}: branch on unknown {:?} falls through", s.offset, c)); false } };
                    if taken { match jump(*target as usize, &mut self.run.notes) { Some(i) => pc = i, None => return } }
                }
                Expr::PushExecutionFlow(t) => flow.push(*t as usize),
                Expr::PopExecutionFlow => match flow.pop() { Some(t) => match jump(t, &mut self.run.notes) { Some(i) => pc = i, None => return }, None => return },
                Expr::PopExecutionFlowIfNot(cond) => {
                    let c = self.val(cond);
                    if c.as_bool() == Some(false) { match flow.pop() { Some(t) => match jump(t, &mut self.run.notes) { Some(i) => pc = i, None => return }, None => return } }
                }
                Expr::ComputedJump(_) => {}
                e => self.statement(e, s.offset),
            }
        }
    }

    fn assign(&mut self, var: &Expr, v: Val) {
        match var {
            Expr::LocalVariable(r) | Expr::LocalOutVariable(r) => { self.locals.insert(r.name.clone(), v); }
            Expr::InstanceVariable(r) => { self.vars.insert(r.name.clone(), v); }
            Expr::StructMemberContext { prop, expr } => {
                if let Expr::LocalVariable(r) = &**expr {
                    let mut s = match self.locals.remove(&r.name) { Some(Val::Struct(s)) => s, _ => vec![] };
                    s.retain(|(k, _)| *k != prop.name);
                    s.push((prop.name.clone(), v));
                    self.locals.insert(r.name.clone(), Val::Struct(s));
                }
            }
            Expr::Context { object, context, .. } | Expr::ContextFailSilent { object, context, .. } => {
                // `ctx(V:Widget . V:Prop) = value`: a property of a widget (recorded as a property write)
                if let (Expr::InstanceVariable(o), Expr::InstanceVariable(p)) = (&**object, &**context) { self.run.calls.push(Call { on: Some(o.name.clone()), func: format!("set {}", p.name), args: vec![v], offset: 0 }); }
            }
            _ => {}
        }
    }

    fn statement(&mut self, e: &Expr, offset: usize) {
        match e {
            Expr::Let { var, value, .. } | Expr::LetBool { var, value } | Expr::LetObj { var, value } => { let v = self.val(value); self.assign(var, v); }
            Expr::LetValueOnPersistentFrame { prop, value } => { let v = self.val(value); self.frame.insert(prop.name.clone(), v); }
            Expr::SetArray { array, elements } => { let v = Val::Array(elements.iter().map(|x| self.val(x)).collect()); self.assign(array, v); }
            Expr::Context { object, context, .. } | Expr::ContextFailSilent { object, context, .. } => {
                let before = self.run.calls.len();
                let _ = self.context_value(object, context);
                if let Some(c) = self.run.calls.get_mut(before) { c.offset = offset; }
            }
            Expr::VirtualFunction { .. } | Expr::LocalVirtualFunction { .. } | Expr::FinalFunction { .. } | Expr::LocalFinalFunction { .. } | Expr::CallMath { .. } => { let _ = self.val(e); }
            _ => {}
        }
    }
}

/// Loaded-script offset of every statement (research helper, see `Vm::exec`).
pub fn mem_offsets(st: &[Statement]) -> Vec<usize> {
    let mut extra = 0usize;
    st.iter().map(|s| { let o = s.offset + extra; extra += 4 * ptr_count(&s.expr); o }).collect()
}

/// Number of object-pointer operands (4 bytes in the cooked file, 8 in memory) inside an expression tree.
fn ptr_count(e: &Expr) -> usize {
    use Expr::*;
    let own = match e {
        LocalVariable(_) | InstanceVariable(_) | DefaultVariable(_) | LocalOutVariable(_) | ObjectConst(_) | PropertyConst(_) => 1,
        Let { .. } | LetValueOnPersistentFrame { .. } | ClassContext { .. } | Context { .. } | ContextFailSilent { .. } | StructMemberContext { .. } => 1,
        MetaCast { .. } | DynamicCast { .. } | ObjToInterfaceCast { .. } | CrossInterfaceCast { .. } | InterfaceToObjCast { .. } => 1,
        FinalFunction { .. } | LocalFinalFunction { .. } | CallMath { .. } | CallMulticastDelegate { .. } => 1,
        StructConst { .. } | ArrayConst { .. } => 1,
        TextConst(TextLit::StringTable { .. }) => 1,
        // an `FName` operand is 8 bytes in the file and 12 in memory (the cooker's editor build keeps `FName::DisplayIndex`): counts like a pointer
        VirtualFunction { .. } | LocalVirtualFunction { .. } | NameConst(_) | InstanceDelegate(_) | BindDelegate { .. } => 1,
        _ => 0,
    };
    own + e.children().into_iter().map(ptr_count).sum::<usize>()
}

fn same(a: &Val, b: &Val) -> bool {
    match (a.as_int(), b.as_int()) { (Some(x), Some(y)) => x == y, _ => a == b }
}

fn math(name: &str, a: &[Val]) -> Val {
    let b = |i: usize| a.get(i).and_then(|v| v.as_bool());
    let n = |i: usize| a.get(i).and_then(|v| v.as_int());
    let f = |i: usize| a.get(i).and_then(|v| v.as_float());
    let un = || Val::Unknown(format!("math {name}"));
    match name {
        "BooleanOR" => match (b(0), b(1)) { (Some(x), Some(y)) => Val::Bool(x || y), (Some(true), _) | (_, Some(true)) => Val::Bool(true), _ => un() },
        "BooleanAND" => match (b(0), b(1)) { (Some(x), Some(y)) => Val::Bool(x && y), (Some(false), _) | (_, Some(false)) => Val::Bool(false), _ => un() },
        "Not_PreBool" => b(0).map(|x| Val::Bool(!x)).unwrap_or_else(un),
        "Conv_BoolToInt" => b(0).map(|x| Val::Int(x as i64)).unwrap_or_else(un),
        "Conv_IntToBool" => n(0).map(|x| Val::Bool(x != 0)).unwrap_or_else(un),
        "Conv_IntToFloat" | "Conv_ByteToFloat" => n(0).map(|x| Val::Float(x as f32)).unwrap_or_else(un),
        "Add_IntInt" => match (n(0), n(1)) { (Some(x), Some(y)) => Val::Int(x + y), _ => un() },
        "Subtract_IntInt" => match (n(0), n(1)) { (Some(x), Some(y)) => Val::Int(x - y), _ => un() },
        "Less_IntInt" => match (n(0), n(1)) { (Some(x), Some(y)) => Val::Bool(x < y), _ => un() },
        "LessEqual_IntInt" => match (n(0), n(1)) { (Some(x), Some(y)) => Val::Bool(x <= y), _ => un() },
        "Greater_IntInt" => match (n(0), n(1)) { (Some(x), Some(y)) => Val::Bool(x > y), _ => un() },
        "EqualEqual_IntInt" | "EqualEqual_ByteByte" => match (n(0), n(1)) { (Some(x), Some(y)) => Val::Bool(x == y), _ => un() },
        "NotEqual_IntInt" | "NotEqual_ByteByte" => match (n(0), n(1)) { (Some(x), Some(y)) => Val::Bool(x != y), _ => un() },
        "EqualEqual_NameName" => match (a.first().and_then(|v| v.as_name()), a.get(1).and_then(|v| v.as_name())) { (Some(x), Some(y)) => Val::Bool(x == y), _ => un() },
        "NotEqual_NameName" => match (a.first().and_then(|v| v.as_name()), a.get(1).and_then(|v| v.as_name())) { (Some(x), Some(y)) => Val::Bool(x != y), _ => un() },
        "Less_FloatFloat" => match (f(0), f(1)) { (Some(x), Some(y)) => Val::Bool(x < y), _ => un() },
        "Multiply_FloatFloat" => match (f(0), f(1)) { (Some(x), Some(y)) => Val::Float(x * y), _ => un() },
        _ => un(),
    }
}

/// Every call a Blueprint function makes, in execution order, with no natives answered (research helper).
pub fn calls(store: &Store, long: &str, name: &str) -> Option<Vec<Call>> {
    store.function(long, name)?;
    let mut vm = Vm::new(store, long, &NoNatives);
    vm.exec_function(name, None);
    Some(vm.run.calls)
}

/// Resolve a `Val::Text` through `Game.locres` (namespace, key); a keyless text is its literal. `None` when the key is missing from the locres.
pub fn resolve_text(store: &Store, v: &Val) -> Option<String> {
    let Val::Text { src, key, ns } = v else { return None };
    if key.is_empty() { return Some(src.clone()); }
    store.locres().and_then(|l| l.get(ns, key).map(|s| s.to_owned()))
}
