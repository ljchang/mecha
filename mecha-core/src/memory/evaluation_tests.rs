use super::*;

#[test]
fn evaluation_reports_real_baseline_misses_separately_from_oracle_evidence() {
    let f = transcript_tests::Fixture::new();
    let path = f.dir.join("evaluation.memory-prototype.sqlite");
    let report = evaluation::run(&path).unwrap();
    assert_eq!(report.cases.len(), 32);
    assert_eq!(report.oracle_complete, 32);
    assert!(report.baseline_complete > 0 && report.baseline_complete < 32);
    assert!(!report.reasoning_evaluated);
    for case in &report.cases {
        assert!(!case.oracle.is_empty());
        assert!(case
            .oracle
            .iter()
            .all(|h| h.private_data && h.untrusted_input));
        assert!(case.coverage >= 0.0 && case.coverage <= 1.0);
    }
    assert!(
        evaluation::run(&path).is_err(),
        "never overwrite an existing evaluation database"
    );
}
