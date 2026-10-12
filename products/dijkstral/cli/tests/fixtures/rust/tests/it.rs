use shapes::model::{classify, Kind};
use shapes::util::twice;

#[test]
fn classifies_empty() {
    assert_eq!(classify(&Kind::Empty), "other");
    assert_eq!(twice(2), 4);
}
