use sex_js::{Document, Error, Limits, Script};

fn clock() -> u64 {
    use std::time::Instant;
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

fn page() -> Script<Document> {
    Script::new(Document::new(sex_html::parse::parse(""), "https://example.test/", (410, 330), clock),
        Limits { budget_ms: 200, load_budget_ms: 200, ..Limits::default() }).unwrap()
}

#[test]
fn nested_precedence_compiles_without_raising_runtime_depth() {
    let mut s = page();
    let expression = format!("{}1+2*3{}", "(".repeat(96), ")".repeat(96));
    s.eval(&format!("console.log({expression})"), "deep.js").unwrap();
    assert_eq!(s.page_mut().log.drain(..).collect::<Vec<_>>(), [(0, String::from("7"))]);
}

#[test]
fn failed_nested_private_rhs_preserves_exception_and_next_eval() {
    let mut s = page();
    s.eval(r#"
        for (let i=0;i<32;i++) {
            try { eval('class C { #p; f(){ return #p in (#p in (1+*2)) } }') }
            catch(e) { if (!(e instanceof SyntaxError)) throw e; }
        }
        class C { #p; f(x){ return [#p in x, (#p in x)==true, (#p in x)|2] } }
        let c=new C;console.log(JSON.stringify(c.f(c)),JSON.stringify(c.f({})));
    "#, "private.js").unwrap();
    assert_eq!(s.page_mut().log.drain(..).collect::<Vec<_>>(),
        [(0, String::from("[true,true,3] [false,false,2]"))]);
}

#[test]
fn runtime_recursion_is_still_bounded() {
    let mut s = page();
    let r = s.eval("function recursive(){return recursive()} recursive()", "recursive.js");
    assert!(matches!(r, Err(Error::Trapped(_)) | Err(Error::Thrown)), "{r:?}");
}
