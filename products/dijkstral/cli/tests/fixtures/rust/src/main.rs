use shapes::model::{round_kind, Circle};
use shapes::{build, label, render, total_area, Round, Shape};

fn main() {
    let reg = build(&["a", "b"]);
    let c = Circle::new(3.0);
    println!("{} {}", render(&c), label("X"));
    let r = Round::new(1.0);
    let v = vec![r.area(), c.area()];
    let n = shapes::util::run_all();
    let total: f64 = v.iter().sum();
    eprintln!("{} {} {}", reg.len(), n, total);
    let area = total_area(&[]);
    let k = shapes::model::classify(&shapes::model::Kind::Empty);
    let r2 = round_kind();
    eprintln!("{} {} {:?}", area, k, r2);
    missing_function();
}
