use crate::models::profile::*;
use crate::parser::operator_parser::OperatorInfo;
use crate::parser::value_parser;
use once_cell::sync::Lazy;
use regex::Regex;

static FRAGMENT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"Fragment\s+(\d+)\s*:").unwrap());

// Doris 3.0: "Pipeline : 0(instance_num=1):"
// Doris 4.0: "Pipeline 0(instance_num=1):"
static PIPELINE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"Pipeline\s*:?\s*(\d+)\s*\(instance_num=(\d+)\)\s*:").unwrap());

// Counter line: "- MetricName: avg VALUE, max VALUE, min VALUE"
// Or: "- MetricName: sum VALUE, avg VALUE, max VALUE, min VALUE"
static COUNTER_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*-\s+(\S[^:]*?)\s*:\s+(.+)$").unwrap());

// PlanInfo block
static PLAN_INFO_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*-\s+PlanInfo\s*$").unwrap());

static PLAN_INFO_ITEM_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*-\s+(.+?):\s+(.+)$").unwrap());

/// Parse the MergedProfile section into Fragments.
pub fn parse_merged_profile(text: &str) -> Vec<Fragment> {
    let lines: Vec<&str> = text.lines().collect();
    let mut fragments = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        if let Some(caps) = FRAGMENT_RE.captures(line) {
            let frag_id: i32 = caps[1].parse().unwrap_or(0);
            let (fragment, next_i) = parse_fragment(frag_id, &lines, i + 1);
            fragments.push(fragment);
            i = next_i;
        } else {
            i += 1;
        }
    }

    fragments
}

fn parse_fragment(frag_id: i32, lines: &[&str], start: usize) -> (Fragment, usize) {
    let mut pipelines = Vec::new();
    let mut i = start;

    let _frag_indent = get_indent(lines.get(start.saturating_sub(1)).unwrap_or(&""));

    while i < lines.len() {
        let line = lines[i];

        // Check if we've hit the next fragment
        if FRAGMENT_RE.is_match(line) {
            break;
        }

        if let Some(caps) = PIPELINE_RE.captures(line) {
            let pipe_id: i32 = caps[1].parse().unwrap_or(0);
            let instance_num: i32 = caps[2].parse().unwrap_or(1);
            let (pipeline, next_i) = parse_pipeline(pipe_id, instance_num, lines, i + 1);
            pipelines.push(pipeline);
            i = next_i;
        } else {
            i += 1;
        }
    }

    (
        Fragment {
            id: frag_id,
            pipelines,
        },
        i,
    )
}

fn parse_pipeline(
    pipe_id: i32,
    instance_num: i32,
    lines: &[&str],
    start: usize,
) -> (Pipeline, usize) {
    let mut operators = Vec::new();
    let mut i = start;

    while i < lines.len() {
        let line = lines[i];

        // Stop at next pipeline or fragment
        if PIPELINE_RE.is_match(line) || FRAGMENT_RE.is_match(line) {
            break;
        }

        // Try to parse operator header
        if let Some(mut op_info) = OperatorInfo::parse(line) {
            let (operator, next_i) = parse_operator(&mut op_info, lines, i + 1);
            operators.push(operator);
            i = next_i;
        } else {
            i += 1;
        }
    }

    (
        Pipeline {
            id: pipe_id,
            instance_num,
            operators,
        },
        i,
    )
}

fn parse_operator(info: &mut OperatorInfo, lines: &[&str], start: usize) -> (Operator, usize) {
    let mut counters = std::collections::HashMap::new();
    let mut plan_info = std::collections::HashMap::new();
    let mut i = start;
    let mut in_plan_info = false;
    let mut plan_info_indent = 0usize;

    while i < lines.len() {
        let line = lines[i];

        // Stop if we hit another operator, pipeline, or fragment
        if OperatorInfo::parse(line).is_some()
            || PIPELINE_RE.is_match(line)
            || FRAGMENT_RE.is_match(line)
        {
            break;
        }

        // Check for PlanInfo block
        if PLAN_INFO_RE.is_match(line) {
            in_plan_info = true;
            plan_info_indent = get_indent(line);
            i += 1;
            continue;
        }

        if in_plan_info {
            // PlanInfo items are rendered as children of the "- PlanInfo" node, so
            // they are indented deeper than it. The block ends the moment a line
            // dedents back to the header's indent or shallower: that line is the
            // operator's first counter (Doris 5, where counters sit at the PlanInfo
            // level) or a CommonCounters:/CustomCounters: group header (Doris 3/4).
            // We key off indentation, not line shape, on purpose — counter lines
            // like "- RowsProduced: sum 4, avg 4, max 4, min 4" share the exact
            // "- key: value" shape of plan-info items, so a shape-based check
            // swallows every counter into plan_info and nulls out output_rows.
            if line.trim().is_empty() {
                // Blank lines don't terminate the block.
                i += 1;
                continue;
            }
            if get_indent(line) > plan_info_indent {
                if let Some(caps) = PLAN_INFO_ITEM_RE.captures(line) {
                    let key = caps[1].trim().to_string();
                    let value = caps[2].trim().to_string();

                    // Check if this is a table reference in a scan operator
                    // Doris format: "TABLE: tpch_demo.orders(orders), PREAGGREGATION: ON"
                    if key == "TABLE" || key == "table" {
                        let table_val = value
                            .split(',')
                            .next()
                            .unwrap_or(&value)
                            .split('(')
                            .next()
                            .unwrap_or(&value)
                            .trim()
                            .to_string();
                        if !table_val.is_empty() {
                            info.table_name = Some(table_val);
                        }
                    }

                    plan_info.insert(key, value);
                }
                // Deeper lines that aren't "key: value" (e.g. "- local merge sort",
                // "- cardinality=4") are plan-info prose we don't structure — skip
                // them but stay inside the block.
                i += 1;
                continue;
            }

            // Dedented to the header level or shallower: the PlanInfo block is over.
            // Fall through and reprocess this same line as a counter / group header.
            in_plan_info = false;
        }

        if !in_plan_info {
            let trimmed = line.trim();
            // Skip Doris 4.0 subsection headers
            if trimmed == "CommonCounters:" || trimmed == "CustomCounters:" {
                i += 1;
                continue;
            }

            if let Some(caps) = COUNTER_RE.captures(line) {
                let name = caps[1].trim().to_string();
                let value_str = caps[2].trim().to_string();

                if let Some(agg) = parse_agg_value(&value_str) {
                    counters.insert(name, agg);
                }
            }
        }

        i += 1;
    }

    // Build OperatorMetrics from known counters
    let metrics = OperatorMetrics {
        exec_time: counters.get("ExecTime").cloned(),
        input_rows: counters.get("InputRows").cloned(),
        rows_produced: counters.get("RowsProduced").cloned(),
        memory_usage: counters.get("MemoryUsage").cloned(),
        memory_usage_peak: counters.get("MemoryUsagePeak").cloned(),
        open_time: counters.get("OpenTime").cloned(),
        close_time: counters.get("CloseTime").cloned(),
        init_time: counters.get("InitTime").cloned(),
        wait_for_dependency_time: counters.get("WaitForDependencyTime").cloned(),
    };

    let op = Operator {
        info: crate::models::profile::OperatorInfoModel {
            full_name: info.full_name.clone(),
            operator_type: info.operator_type.clone(),
            id: info.id,
            nereids_id: info.nereids_id,
            table_name: info.table_name.clone(),
            dest_id: info.dest_id,
            is_sink: info.is_sink,
        },
        metrics,
        all_counters: counters,
        plan_info,
    };

    (op, i)
}

/// Parse an aggregated value string like "avg 150.364us, max 150.364us, min 150.364us"
/// or "sum 24, avg 24, max 24, min 24"
fn parse_agg_value(s: &str) -> Option<AggValue> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let mut sum = None;
    let mut avg = None;
    let mut max = None;
    let mut min = None;

    // Split by comma and parse each part
    for part in s.split(',') {
        let part = part.trim();
        if let Some(val_str) = part
            .strip_prefix("sum ")
            .or_else(|| part.strip_prefix("sum\t"))
        {
            sum = Some(value_parser::parse_counter_value(val_str));
        } else if let Some(val_str) = part
            .strip_prefix("avg ")
            .or_else(|| part.strip_prefix("avg\t"))
        {
            avg = Some(value_parser::parse_counter_value(val_str));
        } else if let Some(val_str) = part
            .strip_prefix("max ")
            .or_else(|| part.strip_prefix("max\t"))
        {
            max = Some(value_parser::parse_counter_value(val_str));
        } else if let Some(val_str) = part
            .strip_prefix("min ")
            .or_else(|| part.strip_prefix("min\t"))
        {
            min = Some(value_parser::parse_counter_value(val_str));
        } else {
            // Single value (no prefix) — treat as all fields
            let v = value_parser::parse_counter_value(part);
            if v != 0.0 || part == "0" || part == "0ns" || part.starts_with("0.") {
                return Some(AggValue {
                    sum: Some(v),
                    avg: Some(v),
                    max: Some(v),
                    min: Some(v),
                    raw: Some(s.to_string()),
                });
            }
        }
    }

    if avg.is_some() || sum.is_some() || max.is_some() || min.is_some() {
        Some(AggValue {
            sum,
            avg,
            max,
            min,
            raw: Some(s.to_string()),
        })
    } else {
        None
    }
}

fn get_indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_agg_value_time() {
        let agg = parse_agg_value("avg 150.364us, max 150.364us, min 150.364us").unwrap();
        assert!((agg.avg.unwrap() - 0.150364).abs() < 0.001);
    }

    #[test]
    fn test_parse_agg_value_with_sum() {
        let agg = parse_agg_value("sum 24, avg 24, max 24, min 24").unwrap();
        assert!((agg.sum.unwrap() - 24.0).abs() < 0.001);
        assert!((agg.avg.unwrap() - 24.0).abs() < 0.001);
    }

    #[test]
    fn test_parse_agg_value_bytes() {
        let agg =
            parse_agg_value("sum 57.94 KB, avg 57.94 KB, max 57.94 KB, min 57.94 KB").unwrap();
        assert!(agg.sum.unwrap() > 50000.0);
    }

    // WHY: the diagnosis layer's output_rows / total_scan_rows are read straight
    // off each operator's RowsProduced counter. An operator that carries a
    // "- PlanInfo" block must still surface its counters; if the PlanInfo block
    // fails to terminate it swallows every following "- key: value" counter line
    // and the operator silently loses output_rows. These two tests pin the
    // termination for both profile layouts so that regression can't return.

    #[test]
    fn planinfo_block_does_not_swallow_counters_doris4() {
        // Doris 3/4 layout: a "- PlanInfo" block, then CommonCounters/CustomCounters
        // group headers holding the real counters (counters indented deeper than the
        // group header, the group header shallower than the PlanInfo header).
        let text = "\
Fragment 0:
  Pipeline 0(instance_num=1):
    AGGREGATION_OPERATOR(nereids_id=153)(id=3):
       - PlanInfo
          - output: count(partial_count(*))[#9]
          - group by: event_type
          - cardinality=4
      CommonCounters:
         - BlocksProduced: sum 4, avg 0, max 1, min 0
         - RowsProduced: sum 7, avg 0, max 1, min 0
      CustomCounters:
         - GetBlockFailedTime: sum 2, avg 0, max 1, min 0
";
        let frags = parse_merged_profile(text);
        let op = &frags[0].pipelines[0].operators[0];
        assert_eq!(
            op.metrics.rows_produced.as_ref().and_then(|v| v.sum),
            Some(7.0),
            "RowsProduced must survive a preceding PlanInfo block"
        );
        // PlanInfo itself is still captured...
        assert!(op.plan_info.contains_key("group by"));
        // ...and a plan-info scalar must not leak into the counters map.
        assert!(!op.all_counters.contains_key("output"));
    }

    #[test]
    fn planinfo_block_does_not_swallow_counters_doris5() {
        // Doris 5 layout: no CommonCounters/CustomCounters grouping — the operator's
        // counters sit directly under it at the same indent as the "- PlanInfo"
        // header. The block must still terminate so RowsProduced is captured (this is
        // the case in the bug report where only PlanInfo-less operators populated).
        let text = "\
Fragment 0:
  Pipeline 0(instance_num=1):
    OLAP_SCAN_OPERATOR(nereids_id=160. table_name=sales(sales))(id=0):
       - PlanInfo
          - TABLE: db.sales(sales)
          - PREDICATES: amount > 10
       - BlocksProduced: sum 4, avg 4, max 4, min 4
       - RowsProduced: sum 450, avg 450, max 450, min 450
       - ScanRows: sum 500, avg 500, max 500, min 500
";
        let frags = parse_merged_profile(text);
        let op = &frags[0].pipelines[0].operators[0];
        assert_eq!(
            op.metrics.rows_produced.as_ref().and_then(|v| v.sum),
            Some(450.0),
            "RowsProduced must be captured when counters sit at the PlanInfo indent"
        );
        assert_eq!(
            op.all_counters.get("ScanRows").and_then(|v| v.sum),
            Some(500.0)
        );
        // PlanInfo's TABLE is still mined for the table name.
        assert_eq!(op.info.table_name.as_deref(), Some("db.sales"));
    }
}
