//! {{name}}: describe what it does here.
//!   cargo run -- a b --json
use std::env;

fn run(args: &[String]) -> (String, i32) {
    let json = args.iter().any(|a| a == "--json");
    let items: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    if json {
        let list = items.iter().map(|s| format!("\"{}\"", s.replace('"', "\\\""))).collect::<Vec<_>>().join(",");
        (format!("{{\"items\":[{}],\"count\":{}}}", list, items.len()), 0)
    } else {
        (format!("{} item(s): {}", items.len(), items.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" ")), 0)
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let (out, code) = run(&args);
    println!("{}", out);
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::run;
    #[test]
    fn counts_items() {
        let (out, code) = run(&["a".into(), "b".into(), "--json".into()]);
        assert_eq!(code, 0);
        assert!(out.contains("\"count\":2"), "{}", out);
    }
}
