//! Parity with Graphify's clustered golden for `tests/fixtures/powershell`, plus probes of the
//! PowerShell rules (docs/graphify-format.md §41).

mod common;
mod probe;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// None: the fixture is reproduced exactly.
const ALLOWED: &[&str] = &[];

#[test]
fn powershell_fixture_matches_graphify() {
    common::assert_fixture("powershell", ALLOWED, common::Communities::Exact, true);
}

#[test]
fn imports_take_bare_stems_and_using_takes_the_last_segment() {
    let l = probe::links(&[(
        "a/main.ps1",
        "Import-Module Az.Accounts\nImport-Module './quoted.psm1'\n. ./lib/dot.name.ps1\n. $PSScriptRoot\\var.ps1\nusing namespace System.Windows.Forms\nusing module ./mods/Deep.psm1\n& ./amp.ps1\nfunction F { Import-Module InFn }\nclass K { [void] M() { Import-Module InMethod } }\n",
    )]);
    for want in [
        "a_main -imports_from-> az",
        "a_main -imports_from-> dot_name",
        "a_main -imports_from-> forms",
        "a_main -imports_from-> psm1",
        "a_main -imports_from-> infn",
    ] {
        assert!(l.contains(want), "missing {want}: {l:#?}");
    }
    assert_eq!(
        l.iter().filter(|x| x.contains("imports_from")).count(),
        5,
        "{l:#?}"
    );
}

#[test]
fn calls_bind_in_file_case_insensitively_and_across_files_by_exact_name() {
    let l = probe::links(&[
        (
            "a/main.ps1",
            "function Caller {\n    leaf\n    & Leaf\n    1..3 | ForEach-Object { Shared-Fn }\n    unique-low\n    Describe\n}\nfunction Leaf { }\nclass K {\n    [void] M() { $this.H(); [K]::S() }\n    [void] H() { }\n    static [void] S() { }\n}\n",
        ),
        ("a/near.ps1", "function Shared-Fn { }\n"),
        (
            "b/far.ps1",
            "function Shared-Fn { }\nfunction Unique-Low { }\nclass Other { [string] Describe() { return '' } }\n",
        ),
    ]);
    for want in [
        "a_main_caller -calls-> a_main_leaf",
        "a_main_caller -calls-> a_near_shared_fn [call]",
        "a_main_caller -calls-> b_far_other_describe [call]",
        "a_main_k_m -calls-> a_main_k_h",
        "a_main_k_m -calls-> a_main_k_s",
    ] {
        assert!(l.contains(want), "missing {want}: {l:#?}");
    }
    // Cross-file names are case-sensitive.
    assert!(
        !l.iter()
            .any(|x| x.contains("unique_low") && x.contains("calls")),
        "{l:#?}"
    );
}

#[test]
fn types_bind_in_file_then_to_the_only_powershell_class() {
    let l = probe::links(&[
        (
            "a.ps1",
            "class K : Base, IFace {\n    [K]$Self\n    [Other]$O\n    [string[]]$Tags\n    [System.IO.FileInfo]$F\n    [K] Clone([Py]$p) { return $null }\n}\nclass Base { }\n",
        ),
        ("b.ps1", "class Other { }\n"),
        ("c.py", "class Py:\n    pass\n"),
    ]);
    for want in [
        "a_k -inherits-> a_base",
        "a_k -implements-> iface",
        "a_k -references-> b_other [field]",
        "a_k -references-> fileinfo [field]",
        "a_k_clone -references-> py [parameter_type]",
    ] {
        assert!(l.contains(want), "missing {want}: {l:#?}");
    }
    // A property never references its own class (`[K]$Self`); the return type `[K]` does, but
    // that link folds into `method` (one link per pair, §8.2).
    assert!(!l.contains("a_k -references-> a_k [field]"), "{l:#?}");
}
