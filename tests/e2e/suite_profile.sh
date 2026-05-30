# shellcheck shell=bash
# suite_profile — query profile analysis, tested against REAL profiles.
#
# Philosophy: send real SQL, fetch the REAL profile over the FE HTTP API, and
# assert on the PARSED VALUES — not just the JSON shape. The full profile text is
# available ONLY over HTTP (see fetch.rs: REST v2 / legacy; the SQL path yields
# summary metadata with no operators). So the FE HTTP profile API is a HARD
# requirement here: if it is unreachable the parser surface cannot be exercised
# at all, and we FAIL loudly rather than SKIP — a silent skip behind a green run
# would hide the entire parser.
#
# Dependency map (from the source):
#   profile list / list --active   MySQL only                   (always testable)
#   profile get <id> (summary)     HTTP profile text -> parsed  (FAIL if HTTP down)
#   profile get --full / --raw     HTTP profile text -> parsed  (FAIL if HTTP down)
#   profile diff                   HTTP profile text -> parsed  (FAIL if HTTP down)
#   profile history                __internal_schema.audit_log  (SKIP if absent)
#
# A profile that was evicted before we could fetch it (FE retains a limited
# number) is a cluster precondition, not a parser bug -> SKIP, distinguished from
# HTTP-down (FAIL) and from a genuinely wrong parse (FAIL).
#
# HTTP_OK is set in preflight from `auth status`.http_status.

suite_profile() {
  suite_banner "profile  (real-profile parse: list / get / full / raw / diff / history)"
  DCLI_STATELESS=1

  # ── Generate REAL profiled queries and capture their ids ────────────────
  # A, A2: the SAME group-by over seeded `events`, run twice. A drives the
  # get/full/raw value assertions; (A, A2) drive diff (same operators match).
  # J: a hash join events⨝dim_users, to exercise join-operator + multi-scan.
  local gb_sql="SELECT event_type, COUNT(*) AS c FROM \`$CFG_DB\`.events GROUP BY event_type ORDER BY c DESC"
  local join_sql="SELECT e.event_type, COUNT(u.name) AS c FROM \`$CFG_DB\`.events e JOIN \`$CFG_DB\`.dim_users u ON e.user_id = u.user_id GROUP BY e.event_type"

  DCLI_STATELESS=1 _run_dcli --format json sql "$gb_sql" --profile
  local qid_a; qid_a="$(jget '.query_id')"
  DCLI_STATELESS=1 _run_dcli --format json sql "$gb_sql" --profile --no-cache
  local qid_a2; qid_a2="$(jget '.query_id')"
  DCLI_STATELESS=1 _run_dcli --format json sql "$join_sql" --profile
  local qid_j; qid_j="$(jget '.query_id')"
  log "  ${C_DIM}profiled ids: A=$qid_a A2=$qid_a2 JOIN=$qid_j  (HTTP_OK=$HTTP_OK)${C_RESET}"

  # ── profile list -> JSON array carrying the real SHOW QUERY PROFILE fields ─
  DCLI_STATELESS=1 _run_dcli --format json profile list
  if [ "$RC" -ne 0 ]; then
    record_fail "profile: list returns an array" "exit=$RC; $ERR"
  elif printf '%s' "$OUT" | jq -e 'type=="array"' >/dev/null 2>&1; then
    record_pass "profile: list returns an array"
    expect_parsed "profile: list entries carry query_id/sql/total_time/state" \
      'length==0 or (.[0]|has("query_id") and has("sql") and has("total_time") and has("state"))'
    if [ -n "$qid_a" ] && printf '%s' "$OUT" | jq -e --arg q "$qid_a" 'any(.[]?; .query_id==$q)' >/dev/null 2>&1; then
      record_pass "profile: list includes the just-profiled query"
    else
      record_skip "profile: list includes the just-profiled query" "not retained in SHOW QUERY PROFILE (eviction/timing)"
    fi
  else
    record_fail "profile: list returns an array" "not a JSON array: $(_oneline "$OUT")"
  fi

  # profile list --active -> JSON array (usually empty).
  expect_json "profile: list --active returns an array" 'type=="array"' \
    profile list --active

  # ── HARD GATE: real profile parsing needs the FE HTTP profile API ───────
  if [ "$HTTP_OK" != "1" ]; then
    record_fail "profile: FE HTTP profile API reachable (required to fetch real profiles)" \
      "http_status != connected — cannot fetch profile text to parse. Cloud deployments use --http-port 8080; self-hosted Doris uses 8030. Run 'doriscli auth status' to verify."
  fi

  if [ -z "$qid_a" ]; then
    record_fail "profile: 'sql --profile' produced a query_id to fetch" \
      "no query_id returned; cannot exercise the parser"
  elif [ "$HTTP_OK" != "1" ]; then
    record_fail "profile: get parses the real profile (BLOCKED)" \
      "FE HTTP profile API unreachable — see the gate failure above"
  else
    # ── Default get: parse the REAL profile, assert VALUES ────────────────
    DCLI_STATELESS=1 _run_dcli --format json profile get "$qid_a"
    if [ "$RC" -ne 0 ]; then
      record_fail "profile: get parses the real profile" "exit=$RC; $ERR"
    elif [ "$(jget '.operators|length')" = "0" ] && [ -n "$(jget '.note')" ]; then
      # HTTP is up, but THIS profile wasn't fetchable (evicted/timing): the
      # command fell back to the SQL summary (operators:[] + a note). That is a
      # cluster-retention precondition, not a parser bug -> SKIP the parse tests.
      local why; why="$(jget '.note' | cut -c1-90)"
      record_skip "profile: get parses the real profile" "profile not fetchable now (fallback: $why) — evicted/timing; raise max_query_profile_num on the FEs and re-run"
    else
      # query_id round-trips: the parsed Profile ID equals the id we asked for.
      expect_parsed "profile: get -> summary.query_id matches the sent query" \
        --arg q "$qid_a" '.summary.query_id==$q'
      # The parsed SQL is the group-by we sent.
      expect_parsed "profile: get -> summary.sql is the real query text" \
        '(.summary.sql|type=="string") and (.summary.sql|test("event_type"))'
      # total_time parsed out of the profile text as a positive number of ms.
      expect_parsed "profile: get -> total_time_ms parsed as a positive number" \
        '(.summary.total_time_ms|type=="number") and (.summary.total_time_ms>0)'
      # Flattened operator tree is non-empty and includes a SCAN and an AGG.
      expect_parsed "profile: get -> operators include a SCAN and an AGGREGATION" \
        '(.operators|length>0) and (any(.operators[]; .name|test("SCAN"))) and (any(.operators[]; .name|test("AGG")))'
      # query_stats: scanned rows ≈ what we loaded; fragment/operator counts real.
      expect_parsed "profile: get -> query_stats.total_scan_rows ≈ loaded rows" \
        '(.query_stats.total_scan_rows >= ('"${ROWS:-2000}"' * 0.9 | floor)) and (.query_stats.fragment_count>=1) and (.query_stats.operator_count>0)'
      # Regression (PlanInfo block swallowed per-operator counters): output_rows must
      # populate for operators that carry a "- PlanInfo" block — this is NOT scan
      # specific. The bug nulled RowsProduced -> output_rows for ~every PlanInfo
      # bearing operator (and on Doris 5.0 the OLAP_SCAN itself carries a PlanInfo
      # block, so total_scan_rows above goes null too). AGGREGATION always carries a
      # PlanInfo block, so it must surface a non-null output_rows here.
      expect_parsed "profile: get -> AGGREGATION operator surfaces output_rows (PlanInfo kept its counters)" \
        'any(.operators[]; (.name|test("AGG")) and (.output_rows != null))'
      # Fragment breakdown must be REAL: exactly the 3 fragments of this group-by,
      # count consistent with the array, and NO empty duplicates (regression guard —
      # a DetailProfile/Appendix block bleeding into MergedProfile used to fabricate
      # empty fragments and inflate fragment_count).
      expect_parsed "profile: get -> fragment_count is real (3, no empty duplicates)" \
        '(.query_stats.fragment_count==3) and (.query_stats.fragment_count==(.fragments|length)) and (.fragments|all(.pipelines>0 and has("id") and has("exec_time_ms") and has("instances")))'
      # The Physical Plan section is extracted (regression: header is spelled
      # "PhysicalPlan" without a space on some Doris versions).
      expect_parsed "profile: get -> physical_plan section is extracted" \
        '(.physical_plan|type=="string") and ((.physical_plan|length)>0)'
      # Changed session variables are parsed (regression: JSON-array form + the
      # "ChangedSessionVariables" header spelling). A --profile run always changes
      # at least enable_profile, so this is deterministically non-empty.
      expect_parsed "profile: get -> changed_session_vars parsed (incl. enable_profile)" \
        '(.changed_session_vars|type=="array") and ((.changed_session_vars|length)>0) and (any(.changed_session_vars[]; .name=="enable_profile"))'
      # Table attribution is Doris-4.0+ (operator header carries table_name=...).
      if printf '%s' "$OUT" | jq -e 'any(.operators[]; .table != null)' >/dev/null 2>&1; then
        expect_parsed "profile: get -> a scan operator names the events table" \
          'any(.operators[]; (.table // "")|test("events"))'
      else
        record_skip "profile: get -> a scan operator names the events table" "operator headers carry no table_name (pre-4.0 Doris)"
      fi

      # ── --full: the complete parsed tree (fragments->pipelines->operators) ─
      DCLI_STATELESS=1 _run_dcli --format json profile get "$qid_a" --full
      if [ "$RC" -ne 0 ]; then
        record_fail "profile: get --full parses the full tree" "exit=$RC; $ERR"
      else
        expect_parsed "profile: --full -> profile.summary.query_id matches" \
          --arg q "$qid_a" '.profile.summary.query_id==$q'
        expect_parsed "profile: --full -> fragments->pipelines->operators are populated" \
          '(.profile.fragments|length>0) and (.profile.fragments[0].pipelines|length>0) and (.operators|length>0)'
        expect_parsed "profile: --full -> an operator exposes parsed all_counters" \
          'any(.profile.fragments[].pipelines[].operators[]; (.all_counters|type=="object") and (.all_counters|length>0))'
        # Regression (PlanInfo block swallowed counters): the block must terminate and
        # NOT absorb the counter lines that follow it. The fingerprint of that bug is
        # an operator with a non-empty plan_info but an empty/absent all_counters
        # (every counter, RowsProduced included, got mis-parsed as plan-info). The
        # --full tree exposes both maps per operator, so assert there are none.
        expect_parsed "profile: --full -> no PlanInfo-bearing operator lost its counters" \
          '[.profile.fragments[].pipelines[].operators[] | select((.plan_info|length)>0) | select((.all_counters|length)==0)] | length == 0'
      fi

      # ── --raw: the raw profile text round-trips; capture it as the fixture ─
      DCLI_STATELESS=1 _run_dcli --format json profile get "$qid_a" --raw
      if [ "$RC" -ne 0 ]; then
        record_fail "profile: get --raw returns the raw profile text" "exit=$RC; $ERR"
      elif printf '%s' "$OUT" | jq -e 'type=="string" and test("Summary") and test("MergedProfile")' >/dev/null 2>&1; then
        record_pass "profile: get --raw returns the raw profile text"
        _capture_profile_fixture
      else
        record_fail "profile: get --raw returns the raw profile text" "not a profile text: $(_oneline "$OUT")"
      fi

      # ── JOIN query: a hash join + both tables scanned ─────────────────────
      if [ -z "$qid_j" ]; then
        record_fail "profile: join query produced a query_id" "join --profile returned no query_id"
      else
        DCLI_STATELESS=1 _run_dcli --format json profile get "$qid_j"
        if [ "$RC" -ne 0 ]; then
          record_fail "profile: get(join) parses the join profile" "exit=$RC; $ERR"
        elif [ "$(jget '.operators|length')" = "0" ] && [ -n "$(jget '.note')" ]; then
          record_skip "profile: get(join) parses the join profile" "join profile not fetchable now (evicted/timing)"
        else
          expect_parsed "profile: join -> a JOIN operator is parsed" \
            'any(.operators[]; .name|test("JOIN"))'
          expect_parsed "profile: join -> both tables are scanned (>=2 SCAN operators)" \
            '([.operators[] | select(.name|test("SCAN"))] | length) >= 2'
        fi
      fi

      # ── diff: two REAL runs of the same query ─────────────────────────────
      if [ -z "$qid_a2" ]; then
        record_fail "profile: second run produced a query_id for diff" "no qid_a2"
      else
        DCLI_STATELESS=1 _run_dcli --format json profile diff "$qid_a" "$qid_a2"
        if [ "$RC" -ne 0 ]; then
          record_fail "profile: diff compares two real runs" "exit=$RC; $ERR"
        else
          expect_parsed "profile: diff -> slow/fast carry parsed totals + numeric time_ratio" \
            --arg s "$qid_a" --arg f "$qid_a2" \
            '(.slow.query_id==$s) and (.fast.query_id==$f) and (.slow.operator_count>0) and (.fast.operator_count>0) and (.time_ratio|type=="number")'
          expect_parsed "profile: diff -> operator_diffs is an array" \
            '.operator_diffs|type=="array"'
        fi
      fi
    fi
  fi

  # ── profile history: depends on __internal_schema.audit_log (SKIP if absent)
  DCLI_STATELESS=1 _run_dcli --format json profile history "events" --days 1
  if [ "$RC" -ne 0 ]; then
    if printf '%s' "$ERR" | grep -qi "audit_log"; then
      record_skip "profile: history reads audit_log" "audit_log not enabled/accessible on this cluster"
    else
      record_fail "profile: history reads audit_log" "exit=$RC; $ERR"
    fi
  elif printf '%s' "$OUT" | jq -e 'has("executions") and has("pattern")' >/dev/null 2>&1; then
    record_pass "profile: history reads audit_log"
  else
    record_fail "profile: history reads audit_log" "unexpected shape: $(_oneline "$OUT")"
  fi

  # Negative: an unknown query id cannot be fetched anywhere -> non-zero exit.
  expect_err "profile: get on an unknown id errors" \
    profile get "00000000-0000-0000-0000-000000000000"
}
