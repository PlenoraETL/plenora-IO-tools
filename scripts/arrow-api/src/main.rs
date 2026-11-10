#![forbid(unsafe_code)]
use quote::ToTokens;
use serde_json::{json, Value};
use std::{env, fs, path::Path};
use syn::{visit::Visit, FnArg, Item, UseTree, Visibility};

#[derive(Default)]
struct Paths(Vec<String>);
impl<'ast> Visit<'ast> for Paths {
    fn visit_type_macro(&mut self, _: &'ast syn::TypeMacro) {
        panic!("type macro requires explicit expansion before Arrow API classification");
    }
    fn visit_path(&mut self, p: &'ast syn::Path) {
        self.0.push(p.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>().join("::"));
        syn::visit::visit_path(self, p);
    }
}
fn uses(tree: &UseTree, prefix: &str, out: &mut Vec<Value>) {
    match tree {
        UseTree::Path(p) => uses(&p.tree, &format!("{prefix}{}::", p.ident), out),
        UseTree::Name(n) => out.push(json!([n.ident.to_string(), format!("{prefix}{}", n.ident)])),
        UseTree::Rename(n) => out.push(json!([n.rename.to_string(), format!("{prefix}{}", n.ident)])),
        UseTree::Glob(_) => out.push(json!(["*", prefix.trim_end_matches("::")])),
        UseTree::Group(g) => for t in &g.items { uses(t, prefix, out); },
    }
}
fn test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("cfg") && a.meta.to_token_stream().to_string() == "cfg (test)")
}
fn signature(sig: &syn::Signature, owner: Option<&syn::Type>, inherited: Option<&syn::Generics>) -> Vec<String> {
    let mut p = Paths::default();
    for arg in &sig.inputs {
        match arg {
            FnArg::Typed(t) => p.visit_type(&t.ty),
            FnArg::Receiver(_) => if let Some(t) = owner { p.visit_type(t); },
        }
    }
    p.visit_generics(&sig.generics);
    if let Some(g) = inherited { p.visit_generics(g); }
    if p.0.iter().any(|s| s == "Self" || s.starts_with("Self::")) {
        if let Some(t) = owner { p.visit_type(t); }
    }
    p.0
}
// Bounds are forbidden at public boundaries even when Arrow is not first.
#[derive(Default)]
struct Bounds(Paths);
impl<'ast> Visit<'ast> for Bounds {
    fn visit_type_trait_object(&mut self, _: &'ast syn::TypeTraitObject) {}
    fn visit_type_param_bound(&mut self, bound: &'ast syn::TypeParamBound) {
        self.0.visit_type_param_bound(bound);
    }
}
fn restrictions(sig: &syn::Signature, inherited: &syn::Generics) -> Vec<String> {
    let mut bounds = Bounds::default();
    bounds.visit_signature(sig);
    bounds.visit_generics(inherited);
    bounds.0.0
}
fn walk(items: &[Item], scope: &str, file: &str, out: &mut Vec<Value>) {
    let mut imports = vec![];
    for i in items {
        if let Item::Use(u) = i {
            if test(&u.attrs) { continue; }
            uses(&u.tree, "", &mut imports);
            if matches!(u.vis, Visibility::Public(_)) {
                out.push(json!({"kind":"reexport", "scope":scope,"file":file,
                    "tokens":u.to_token_stream().to_string()}));
                let mut names = vec![];
                uses(&u.tree,"",&mut names);
                for n in names {
                    let alias=n[0].as_str().unwrap(); let source=n[1].as_str().unwrap();
                    if alias != "*" && alias != source.rsplit("::").next().unwrap() {
                        out.push(json!({"kind":"forbidden","id":format!("{scope}::{alias}"),
                            "scope":scope,"file":file,"deps":[source]}));
                    }
                }
            }
        }
        if let Item::ExternCrate(e) = i {
            let alias = e.rename.as_ref().map(|(_,n)|n).unwrap_or(&e.ident);
            imports.push(json!([alias.to_string(), e.ident.to_string()]));
        }
    }
    out.push(json!({"kind":"scope", "scope":scope, "uses":imports}));
    for import in &imports {
        let alias = import[0].as_str().unwrap();
        let source = import[1].as_str().unwrap();
        if alias != "*" && alias != source.rsplit("::").next().unwrap() {
            out.push(json!({"kind":"alias", "id":format!("{scope}::{alias}"),
                "scope":scope,"file":file,"deps":[source]}));
        }
    }
    for i in items {
        if let Item::Macro(m) = i {
            if !test(&m.attrs) {
                out.push(json!({"kind":"opaque", "scope":scope,"file":file,
                    "tokens":m.to_token_stream().to_string()}));
            }
        }
        let (name, attrs) = match i {
            Item::Fn(v) => (v.sig.ident.to_string(), &v.attrs),
            Item::Struct(v) => (v.ident.to_string(), &v.attrs),
            Item::Enum(v) => (v.ident.to_string(), &v.attrs),
            Item::Union(v) => (v.ident.to_string(), &v.attrs),
            Item::Type(v) => (v.ident.to_string(), &v.attrs),
            Item::Trait(v) => (v.ident.to_string(), &v.attrs),
            Item::Impl(v) => (v.self_ty.to_token_stream().to_string().replace(' ', ""), &v.attrs),
            Item::Mod(v) => (v.ident.to_string(), &v.attrs),
            _ => continue,
        };
        if test(attrs) { continue; }
        if attrs.iter().any(|a| a.path().is_ident("pyfunction")) {
            out.push(json!({"kind":"python", "id":format!("plenora_io._native.{name}")}));
        }
        // Additional binding forms require an explicit extractor, never silent omission.
        assert!(!attrs.iter().any(|a| a.path().is_ident("pymethods")), "pymethods requires Python inventory support");
        let id = format!("{scope}::{name}");
        let emit = |out: &mut Vec<Value>, id: String, deps: Vec<String>, kind: &str| {
            out.push(json!({"kind":kind,"id":id,"scope":scope,"file":file,"deps":deps}));
        };
        let mut p = Paths::default();
        let generics = match i {
            Item::Struct(v) if matches!(v.vis,Visibility::Public(_)) => Some(&v.generics),
            Item::Enum(v) if matches!(v.vis,Visibility::Public(_)) => Some(&v.generics),
            Item::Union(v) if matches!(v.vis,Visibility::Public(_)) => Some(&v.generics),
            Item::Trait(v) if matches!(v.vis,Visibility::Public(_)) => Some(&v.generics),
            _ => None,
        };
        if let Some(g) = generics {
            let mut bounds=Bounds::default(); bounds.visit_generics(g);
            if let Item::Trait(v) = i {
                for bound in &v.supertraits { bounds.visit_type_param_bound(bound); }
            }
            emit(out,id.clone(),bounds.0.0,"forbidden");
        }
        if let Item::Fn(v) = i {
            if matches!(v.vis, Visibility::Public(_)) {
                emit(out,id.clone(),restrictions(&v.sig,&v.sig.generics),"forbidden");
            }
        }
        match i {
            Item::Fn(v) if matches!(v.vis, Visibility::Public(_)) => emit(out, id, signature(&v.sig,None,None), "fn"),
            Item::Struct(v) => { p.visit_fields(&v.fields); p.visit_generics(&v.generics); emit(out,id,p.0,"type"); },
            Item::Enum(v) => { for var in &v.variants { p.visit_fields(&var.fields); } p.visit_generics(&v.generics); emit(out,id,p.0,"type"); },
            Item::Union(v) => { p.visit_fields_named(&v.fields); p.visit_generics(&v.generics); emit(out,id,p.0,"type"); },
            Item::Type(v) => {
                p.visit_type(&v.ty); p.visit_generics(&v.generics);
                if matches!(v.vis, Visibility::Public(_)) { emit(out,id.clone(),p.0.clone(),"forbidden"); }
                emit(out,id.clone(),p.0,"type");
                emit(out,id.clone(),vec![id],"alias");
            },
            Item::Trait(v) => {
                // A reader/writer trait transports Arrow through results as well as arguments.
                p.visit_item_trait(v); emit(out,id.clone(),p.0,"type");
                for member in &v.items {
                    if let syn::TraitItem::Type(t) = member {
                        if !test(&t.attrs) {
                            let mut deps = Paths::default();
                            deps.visit_trait_item_type(t);
                            emit(out,format!("{id}::{}",t.ident),deps.0,"forbidden");
                        }
                    }
                    if let syn::TraitItem::Macro(m) = member {
                        if !test(&m.attrs) {
                            out.push(json!({"kind":"opaque", "scope":id,"file":file,
                                "tokens":m.to_token_stream().to_string()}));
                        }
                    }
                    if let syn::TraitItem::Fn(f) = member {
                        if !test(&f.attrs) {
                            emit(out,format!("{id}::{}",f.sig.ident),restrictions(&f.sig,&v.generics),"forbidden");
                            emit(out,format!("{id}::{}",f.sig.ident),signature(&f.sig,Some(&syn::parse_str::<syn::Type>(&name).unwrap()),Some(&v.generics)),"fn");
                        }
                    }
                }
            },
            Item::Impl(v) => for member in &v.items {
                if let syn::ImplItem::Type(t) = member {
                    if !test(&t.attrs) {
                        let mut deps = Paths::default();
                        deps.visit_impl_item_type(t);
                        emit(out,format!("{id}::{}",t.ident),deps.0,"forbidden");
                    }
                }
                if let syn::ImplItem::Macro(m) = member {
                    if !test(&m.attrs) {
                        out.push(json!({"kind":"opaque", "scope":id,"file":file,
                            "tokens":m.to_token_stream().to_string()}));
                    }
                }
                if let syn::ImplItem::Fn(f) = member {
                    if !test(&f.attrs) && (v.trait_.is_some() || matches!(f.vis, Visibility::Public(_))) {
                        emit(out,format!("{id}::{}",f.sig.ident),restrictions(&f.sig,&v.generics),"forbidden");
                        let tr = v.trait_.as_ref().map(|(_,p,_)|format!("[{}]",p.to_token_stream().to_string().replace(' ',""))).unwrap_or_default();
                        emit(out,format!("{id}{tr}::{}",f.sig.ident),signature(&f.sig,Some(&v.self_ty),Some(&v.generics)),"fn");
                    }
                }
            },
            Item::Mod(v) => if let Some((_,children)) = &v.content { walk(children,&id,file,out); },
            _ => (),
        }
    }
}
fn scan(dir: &Path, root: &Path, out: &mut Vec<Value>) {
    let mut entries: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() { if p.file_name().unwrap() != "tests" { scan(&p,root,out); } continue; }
        if p.extension().is_none_or(|e| e != "rs") || p.file_name().unwrap() == "tests.rs" { continue; }
        let relative = p.strip_prefix(root).unwrap().to_str().unwrap().replace('\\',"/");
        if !relative.contains("/src/") { continue; }
        let (krate, module) = relative.split_once("/src/").unwrap();
        let mut parts: Vec<_> = module.trim_end_matches(".rs").split('/').collect();
        if matches!(parts.last(), Some(&"lib" | &"main" | &"mod")) { parts.pop(); }
        let mut scope = krate.replace('-',"_");
        for part in parts { scope.push_str("::"); scope.push_str(part); }
        let syntax = syn::parse_file(&fs::read_to_string(&p).unwrap()).unwrap_or_else(|e|panic!("{relative}: {e}"));
        walk(&syntax.items,&scope,&relative,out);
    }
}
fn main() {
    let arg = env::args().nth(1).expect("crates directory required");
    if arg == "--catalog" {
        // Every public declaration in every source file, across all cfgs.
        // Include pub uses; macro expansion remains a declared static limit.
        struct Catalog(std::collections::BTreeSet<String>);
        impl<'ast> Visit<'ast> for Catalog {
            fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
                let ident = match item {
                    syn::TraitItem::Fn(v) => Some(&v.sig.ident),
                    syn::TraitItem::Type(v) => Some(&v.ident),
                    syn::TraitItem::Const(v) => Some(&v.ident),
                    _ => None,
                };
                if let Some(ident) = ident { self.0.insert(ident.to_string()); }
                syn::visit::visit_trait_item(self,item);
            }
            fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
                let pair = match item {
                    syn::ImplItem::Fn(v) => Some((&v.vis,&v.sig.ident)),
                    syn::ImplItem::Type(v) => Some((&v.vis,&v.ident)),
                    syn::ImplItem::Const(v) => Some((&v.vis,&v.ident)),
                    _ => None,
                };
                if let Some((Visibility::Public(_),ident)) = pair { self.0.insert(ident.to_string()); }
                syn::visit::visit_impl_item(self,item);
            }
            fn visit_variant(&mut self, v: &'ast syn::Variant) {
                self.0.insert(v.ident.to_string());
                syn::visit::visit_variant(self,v);
            }
            fn visit_item(&mut self, item: &'ast Item) {
                let pair = match item {
                    Item::Struct(v) => Some((&v.vis,&v.ident)),
                    Item::Enum(v) => Some((&v.vis,&v.ident)),
                    Item::Union(v) => Some((&v.vis,&v.ident)),
                    Item::Trait(v) => Some((&v.vis,&v.ident)),
                    Item::TraitAlias(v) => Some((&v.vis,&v.ident)),
                    Item::Type(v) => Some((&v.vis,&v.ident)),
                    Item::Fn(v) => Some((&v.vis,&v.sig.ident)),
                    Item::Const(v) => Some((&v.vis,&v.ident)),
                    Item::Static(v) => Some((&v.vis,&v.ident)),
                    Item::Mod(v) => Some((&v.vis,&v.ident)),
                    _ => None,
                };
                if let Some((Visibility::Public(_), ident)) = pair { self.0.insert(ident.to_string()); }
                if let Item::Use(v) = item {
                    if matches!(v.vis,Visibility::Public(_)) {
                        let mut names=vec![]; uses(&v.tree,"",&mut names);
                        for n in names { self.0.insert(n[0].as_str().unwrap().to_owned()); }
                    }
                }
                syn::visit::visit_item(self,item);
            }
        }
        let mut catalog = Catalog(Default::default());
        for file in env::args().skip(2) {
            catalog.visit_file(&syn::parse_file(&fs::read_to_string(&file).unwrap()).unwrap_or_else(|e|panic!("{file}: {e}")));
        }
        println!("{}",serde_json::to_string(&catalog.0).unwrap());
        return;
    }
    let root=Path::new(&arg); let mut out=vec![]; scan(root,root,&mut out);
    println!("{}",serde_json::to_string(&out).unwrap());
}
