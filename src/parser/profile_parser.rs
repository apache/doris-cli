use crate::models::profile::*;
use crate::parser::{fragment_parser, section_parser, value_parser};

/// Parse a raw Doris profile text into a structured DorisProfile.
pub fn parse(raw_text: &str) -> DorisProfile {
    let normalized = section_parser::normalize_text(raw_text);
    let sections = section_parser::split_sections(&normalized);

    // Parse Summary
    let summary_text = sections.get("Summary").map(|s| s.as_str()).unwrap_or("");
    let summary_kv = section_parser::parse_kv_section(summary_text);
    let total_time_str = summary_kv.get("Total").cloned().unwrap_or_default();
    let total_time_ms = value_parser::parse_duration_ms(&total_time_str);

    let summary = ProfileSummary {
        query_id: summary_kv.get("Profile ID").cloned().unwrap_or_default(),
        total_time: total_time_str,
        total_time_ms,
        state: summary_kv.get("Task State").cloned().unwrap_or_default(),
        sql: summary_kv.get("Sql Statement").cloned().unwrap_or_default(),
        doris_version: summary_kv.get("Doris Version").cloned(),
        is_nereids: summary_kv.get("Is Nereids").map(|v| v == "Yes"),
        is_cached: summary_kv.get("Is Cached").map(|v| v == "Yes"),
        total_instances: summary_kv
            .get("Total Instances Num")
            .and_then(|v| v.parse().ok()),
        user: summary_kv.get("User").cloned(),
        start_time: summary_kv.get("Start Time").cloned(),
        end_time: summary_kv.get("End Time").cloned(),
        default_db: summary_kv.get("Default Db").cloned(),
    };

    // Parse Execution Summary
    let exec_text = sections
        .get("Execution Summary")
        .map(|s| s.as_str())
        .unwrap_or("");
    let exec_kv = section_parser::parse_kv_section(exec_text);

    let execution_summary = ExecutionSummary {
        parse_sql_time: exec_kv.get("Parse SQL Time").cloned(),
        plan_time: exec_kv.get("Plan Time").cloned(),
        schedule_time: exec_kv.get("Schedule Time").cloned(),
        wait_fetch_result_time: exec_kv.get("Wait and Fetch Result Time").cloned(),
        fetch_result_time: exec_kv.get("Fetch Result Time").cloned(),
        write_result_time: exec_kv.get("Write Result Time").cloned(),
        nereids_analysis_time: exec_kv.get("Nereids Analysis Time").cloned(),
        nereids_rewrite_time: exec_kv.get("Nereids Rewrite Time").cloned(),
        nereids_optimize_time: exec_kv.get("Nereids Optimize Time").cloned(),
        workload_group: exec_kv.get("Workload Group").cloned(),
    };

    // Parse Changed Session Variables
    let vars_text = sections
        .get("Changed Session Variables")
        .map(|s| s.as_str())
        .unwrap_or("");
    let changed_session_vars = parse_session_vars(vars_text);

    // Parse MergedProfile
    let merged_text = sections
        .get("MergedProfile")
        .map(|s| s.as_str())
        .unwrap_or("");
    let fragments = fragment_parser::parse_merged_profile(merged_text);

    DorisProfile {
        summary,
        execution_summary,
        changed_session_vars,
        fragments,
    }
}

fn parse_session_vars(text: &str) -> Vec<SessionVar> {
    let trimmed = text.trim();

    // Newer Doris emits this section as a JSON array of
    // [{VarName, CurrentValue, DefaultValue}, ...]; older versions use a
    // pipe-delimited table. Try JSON first, then fall back to the table parser.
    if trimmed.starts_with('[') {
        if let Ok(serde_json::Value::Array(items)) =
            serde_json::from_str::<serde_json::Value>(trimmed)
        {
            let vars: Vec<SessionVar> = items
                .iter()
                .filter_map(|item| {
                    let name = item.get("VarName").and_then(|v| v.as_str())?;
                    if name.is_empty() {
                        return None;
                    }
                    Some(SessionVar {
                        name: name.to_string(),
                        current_value: item
                            .get("CurrentValue")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        default_value: item
                            .get("DefaultValue")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                    })
                })
                .collect();
            if !vars.is_empty() {
                return vars;
            }
        }
    }

    // Legacy pipe-delimited table: "VarName | CurrentValue | DefaultValue".
    let mut vars = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        // Skip header and separator lines
        if line.is_empty()
            || line.starts_with("VarName")
            || line.starts_with("---")
            || line.starts_with('-')
        {
            continue;
        }

        let parts: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
        if parts.len() >= 3 {
            vars.push(SessionVar {
                name: parts[0].to_string(),
                current_value: parts[1].to_string(),
                default_value: parts[2].to_string(),
            });
        }
    }
    vars
}

/// Extract all operators from a parsed profile, flattened.
pub fn flatten_operators(profile: &DorisProfile) -> Vec<FlatOperator> {
    let total_time_ms = profile.summary.total_time_ms.unwrap_or(1.0).max(0.001);
    let mut result = Vec::new();

    for fragment in &profile.fragments {
        for pipeline in &fragment.pipelines {
            for op in &pipeline.operators {
                let exec_time_avg_ms = op
                    .metrics
                    .exec_time
                    .as_ref()
                    .and_then(|v| v.avg)
                    .unwrap_or(0.0);

                let input_rows = op.metrics.input_rows.as_ref().and_then(|v| v.sum.or(v.avg));

                let output_rows = op
                    .metrics
                    .rows_produced
                    .as_ref()
                    .and_then(|v| v.sum.or(v.avg));

                let skew_ratio = op.metrics.exec_time.as_ref().and_then(|v| v.skew_ratio());

                let time_pct = (exec_time_avg_ms / total_time_ms) * 100.0;

                // Selectivity: input / output ratio
                let selectivity = match (input_rows, output_rows) {
                    (Some(inp), Some(out)) if out > 0.0 => Some(round2(inp / out)),
                    _ => None,
                };

                // Spill detection
                let spill_write = op
                    .all_counters
                    .get("SpillWriteBytes")
                    .or_else(|| op.all_counters.get("SpillWriteBytesToLocalStorage"))
                    .and_then(|v| v.sum.or(v.avg))
                    .unwrap_or(0.0);
                let spilled = if spill_write > 0.0 { Some(true) } else { None };

                // Peak memory
                let peak_mem_bytes = op
                    .metrics
                    .memory_usage_peak
                    .as_ref()
                    .and_then(|v| v.sum.or(v.avg))
                    .filter(|&v| v > 0.0);

                // Wait dependency analysis: blocked if wait_time > exec_time * 2
                let wait_time_ms = op
                    .metrics
                    .wait_for_dependency_time
                    .as_ref()
                    .and_then(|v| v.avg)
                    .filter(|&v| v > 0.0);

                let blocked_on_upstream = match (wait_time_ms, exec_time_avg_ms) {
                    (Some(wait), exec) if exec > 0.0 && wait > exec * 2.0 => Some(true),
                    _ => None,
                };

                // Runtime filter extraction from plan_info AND all_counters
                let mut runtime_filters: Vec<String> = Vec::new();

                // From plan_info: keys containing RF references
                for (k, v) in &op.plan_info {
                    if k.contains("RF") || k.contains("runtime") || k.contains("RuntimeFilter") {
                        runtime_filters.push(format!("{k}: {v}"));
                    }
                }

                // From all_counters: RuntimeFilter-related counters
                for (k, v) in &op.all_counters {
                    if k.contains("RuntimeFilter") || k.starts_with("RF") {
                        let val = v.avg.or(v.sum).unwrap_or(0.0);
                        if val > 0.0 {
                            runtime_filters.push(format!("{k}: {val}"));
                        }
                    }
                }

                // From all_counters: check for runtime filter specific metrics
                let rf_input = op
                    .all_counters
                    .get("RuntimeFilterInput")
                    .or_else(|| op.all_counters.get("RuntimeFilterNum"))
                    .and_then(|v| v.sum.or(v.avg));
                if let Some(rf_count) = rf_input {
                    if rf_count > 0.0
                        && !runtime_filters.iter().any(|r| r.contains("RuntimeFilter"))
                    {
                        runtime_filters.push(format!("RuntimeFiltersApplied: {rf_count}"));
                    }
                }

                let runtime_filters = if runtime_filters.is_empty() {
                    None
                } else {
                    Some(runtime_filters)
                };

                // Shuffle bytes (for exchange/data_stream operators)
                let shuffle_bytes = op
                    .all_counters
                    .get("ShuffleSendBytes")
                    .or_else(|| op.all_counters.get("BytesSent"))
                    .or_else(|| op.all_counters.get("OverallThroughput"))
                    .and_then(|v| v.sum.or(v.avg))
                    .filter(|&v| v > 0.0);

                // Cache hit percentage (for scan operators)
                let cache_hit_pct = {
                    let hit = op
                        .all_counters
                        .get("FileCacheHitBytes")
                        .or_else(|| op.all_counters.get("CacheHitBytes"))
                        .and_then(|v| v.sum.or(v.avg))
                        .unwrap_or(0.0);
                    let miss = op
                        .all_counters
                        .get("FileCacheMissBytes")
                        .or_else(|| op.all_counters.get("CacheMissBytes"))
                        .and_then(|v| v.sum.or(v.avg))
                        .unwrap_or(0.0);
                    let total = hit + miss;
                    if total > 0.0 {
                        Some(round2((hit / total) * 100.0))
                    } else {
                        None
                    }
                };

                // Rows filtered (scan: output < input means filtering happened)
                let rows_filtered = match (input_rows, output_rows) {
                    (Some(inp), Some(out)) if inp > out && inp > 0.0 => Some(inp - out),
                    _ => None,
                };

                // Join type from PlanInfo
                let join_type = op
                    .plan_info
                    .iter()
                    .find_map(|(k, v)| {
                        let combined = format!("{k} {v}").to_lowercase();
                        if combined.contains("broadcast") {
                            Some("broadcast".to_string())
                        } else if combined.contains("bucket_shuffle")
                            || combined.contains("bucketshuffle")
                        {
                            Some("bucket_shuffle".to_string())
                        } else if combined.contains("colocate") {
                            Some("colocated".to_string())
                        } else if combined.contains("shuffle") || combined.contains("hash") {
                            Some("shuffle".to_string())
                        } else {
                            None
                        }
                    })
                    // Also try extracting from operator name
                    .or_else(|| {
                        let name_lower = op.info.full_name.to_lowercase();
                        if name_lower.contains("broadcast") {
                            Some("broadcast".to_string())
                        } else if name_lower.contains("colocate") {
                            Some("colocated".to_string())
                        } else {
                            None
                        }
                    });

                result.push(FlatOperator {
                    name: op.info.full_name.clone(),
                    table: op.info.table_name.clone(),
                    frag: fragment.id,
                    pipeline: pipeline.id,
                    exec_time_avg_ms: round2(exec_time_avg_ms),
                    input_rows,
                    output_rows,
                    skew_ratio: skew_ratio.map(round2),
                    time_pct: round2(time_pct),
                    selectivity,
                    spilled,
                    peak_mem_bytes,
                    blocked_on_upstream,
                    wait_time_ms: wait_time_ms.map(round2),
                    runtime_filters,
                    shuffle_bytes,
                    cache_hit_pct,
                    rows_filtered,
                    join_type,
                });
            }
        }
    }

    // Sort by exec_time descending
    result.sort_by(|a, b| {
        b.exec_time_avg_ms
            .partial_cmp(&a.exec_time_avg_ms)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    result
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod real_profile_tests {
    use super::*;

    /// A REAL Doris profile captured by the e2e harness: `suite_profile.sh` writes
    /// it on a successful `profile get --raw`. Committing that file turns this into
    /// an offline regression guard for the whole parse pipeline; until it exists the
    /// test is a visible no-op (it prints how to produce the fixture and returns).
    fn fixture_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/e2e/fixtures/sample_profile.txt")
    }

    /// WHY this test exists: the parser's contract is to turn an opaque profile
    /// text into a structured tree the diagnosis commands can rely on. A unit test
    /// over a hand-written snippet can't catch a real-profile-only regression (a
    /// section header that moved, a counter format that changed). This parses an
    /// actual captured profile and asserts the load-bearing invariants every
    /// downstream command depends on — so it FAILS if any of them break.
    #[test]
    fn parses_a_real_captured_profile() {
        let path = fixture_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) if !t.trim().is_empty() => t,
            _ => {
                eprintln!(
                    "SKIP parses_a_real_captured_profile: no fixture at {}.\n  \
                     Run ./start-testing.sh against a cluster, then commit\n  \
                     tests/e2e/fixtures/sample_profile.txt to activate this test.",
                    path.display()
                );
                return;
            }
        };

        let profile = parse(&text);

        // Summary: a real profile must yield a non-empty Profile ID and a positive
        // total time (proves the Summary section + duration parsing both worked).
        assert!(
            !profile.summary.query_id.is_empty(),
            "summary.query_id must be parsed from a real profile"
        );
        assert!(
            profile.summary.total_time_ms.unwrap_or(0.0) > 0.0,
            "summary.total_time_ms must parse to a positive number, got {:?}",
            profile.summary.total_time_ms
        );

        // Structure: at least one fragment with at least one pipeline of operators
        // (proves MergedProfile -> fragment -> pipeline -> operator parsing worked).
        assert!(
            !profile.fragments.is_empty(),
            "a real profile must yield at least one fragment"
        );
        assert!(
            profile
                .fragments
                .iter()
                .any(|f| f.pipelines.iter().any(|p| !p.operators.is_empty())),
            "at least one fragment must contain operators"
        );

        // Flattening: the diagnosis surface must be non-empty, every operator named,
        // and sorted by exec_time descending (the head is the slowest).
        let flat = flatten_operators(&profile);
        assert!(
            !flat.is_empty(),
            "flatten_operators must yield operators for a real profile"
        );
        assert!(
            flat.iter().all(|op| !op.name.is_empty()),
            "every flattened operator must carry a name"
        );
        assert!(
            flat.windows(2)
                .all(|w| w[0].exec_time_avg_ms >= w[1].exec_time_avg_ms),
            "flattened operators must be sorted by exec_time_avg_ms descending"
        );

        // Regression (PlanInfo-swallows-counters): an operator that carries a
        // "- PlanInfo" block must still surface its counters. The bug was that the
        // PlanInfo block never terminated, so every following counter line (which
        // shares the "- key: value" shape of a plan-info item) was parsed as
        // plan-info and the operator lost RowsProduced -> output_rows -> the
        // downstream total_scan_rows degraded to null.
        let with_plan_info: Vec<_> = profile
            .fragments
            .iter()
            .flat_map(|f| &f.pipelines)
            .flat_map(|p| &p.operators)
            .filter(|op| !op.plan_info.is_empty())
            .collect();
        assert!(
            !with_plan_info.is_empty(),
            "fixture must contain operators with a PlanInfo block to exercise this path"
        );
        for op in &with_plan_info {
            // Every Doris operator emits at least ExecTime, so a PlanInfo operator
            // with an empty counter map means the block absorbed the counters.
            assert!(
                !op.all_counters.is_empty(),
                "operator {:?} carries a PlanInfo block but has no counters — the \
                 PlanInfo parser swallowed them",
                op.info.full_name
            );
        }
        assert!(
            with_plan_info
                .iter()
                .any(|op| op.metrics.rows_produced.is_some()),
            "at least one PlanInfo-bearing operator must surface RowsProduced (output_rows)"
        );

        // Regression: DetailProfile / Appendix must be section boundaries, not bleed
        // into MergedProfile and fabricate empty fragments — every parsed fragment has
        // real pipelines.
        assert!(
            profile.fragments.iter().all(|f| !f.pipelines.is_empty()),
            "no parsed fragment may be empty (DetailProfile/Appendix must terminate MergedProfile)"
        );

        // Regression: the Changed Session Variables section is parsed (JSON-array or
        // pipe-table form). A profile captured by the e2e harness ran with --profile,
        // which always changes at least `enable_profile`.
        assert!(
            !profile.changed_session_vars.is_empty(),
            "changed_session_vars must be parsed from a real profile"
        );

        // Regression: the Physical Plan section is recognized regardless of spelling
        // ("PhysicalPlan" vs "Physical Plan").
        let sections = section_parser::split_sections(&section_parser::normalize_text(&text));
        assert!(
            sections.contains_key("Physical Plan"),
            "the Physical Plan section must be extracted"
        );
    }
}
