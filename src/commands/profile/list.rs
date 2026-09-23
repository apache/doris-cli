use crate::config::Environment;
use crate::connection::MysqlConnection;
use serde_json::{json, Value};

fn truncate_sql(sql: &str) -> String {
    if sql.len() > 120 {
        let mut end = 120;
        while !sql.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &sql[..end])
    } else {
        sql.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_sql;

    #[test]
    fn preserves_sql_within_byte_limit() {
        for sql in [
            String::new(),
            "SELECT 1".to_string(),
            "a".repeat(120),
            "断".repeat(40),
        ] {
            assert_eq!(truncate_sql(&sql), sql);
        }
    }

    #[test]
    fn truncates_ascii_at_byte_limit() {
        assert_eq!(
            truncate_sql(&"a".repeat(121)),
            format!("{}...", "a".repeat(120))
        );
    }

    #[test]
    fn truncates_before_multibyte_characters_crossing_byte_limit() {
        for ch in ['é', '断', 'あ', '한', '😀'] {
            for bytes_before_limit in 1..ch.len_utf8() {
                let prefix = "a".repeat(120 - bytes_before_limit);
                let sql = format!("{prefix}{ch} SELECT 1");
                assert_eq!(
                    truncate_sql(&sql),
                    format!("{prefix}..."),
                    "{ch}, {bytes_before_limit}"
                );
            }
        }
    }

    #[test]
    fn retains_multibyte_character_ending_at_byte_limit() {
        let prefix = format!("{}断", "a".repeat(117));
        assert_eq!(truncate_sql(&format!("{prefix}x")), format!("{prefix}..."));
    }

    #[test]
    fn omits_character_starting_at_byte_limit() {
        let prefix = "a".repeat(120);
        assert_eq!(truncate_sql(&format!("{prefix}断")), format!("{prefix}..."));
    }
}

pub async fn run(limit: usize, env: &Environment) -> anyhow::Result<Value> {
    let mut conn = MysqlConnection::connect(env).await?;

    let result = conn.query("SHOW QUERY PROFILE '/'").await?;

    let profiles: Vec<Value> = result
        .rows
        .iter()
        .take(limit)
        .map(|row| {
            let sql = row
                .get("Sql Statement")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            // Truncate SQL for display
            let sql_truncated = truncate_sql(sql);

            // Clean up escaped newlines and collapse whitespace
            let sql_clean = sql_truncated
                .replace("\\n", " ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");

            json!({
                "query_id": row.get("Profile ID").and_then(|v| v.as_str()).unwrap_or(""),
                "sql": sql_clean,
                "total_time": row.get("Total").and_then(|v| v.as_str()).unwrap_or(""),
                "state": row.get("Task State").and_then(|v| v.as_str()).unwrap_or(""),
                "user": row.get("User").and_then(|v| v.as_str()).unwrap_or(""),
                "start_time": row.get("Start Time").and_then(|v| v.as_str()).unwrap_or(""),
                "default_db": row.get("Default Db").and_then(|v| v.as_str()).unwrap_or(""),
            })
        })
        .collect();

    Ok(Value::Array(profiles))
}

/// List currently running queries via information_schema.active_queries.
pub async fn run_active(env: &Environment) -> anyhow::Result<Value> {
    let mut conn = MysqlConnection::connect(env).await?;

    let result = conn
        .query(
            "SELECT * FROM information_schema.active_queries \
             ORDER BY QUERY_TIME_MS DESC",
        )
        .await?;

    let queries: Vec<Value> = result
        .rows
        .iter()
        .map(|row| {
            let sql = row.get("SQL").and_then(|v| v.as_str()).unwrap_or("");
            let sql_truncated = truncate_sql(sql);
            let sql_clean = sql_truncated
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");

            json!({
                "query_id": row.get("QUERY_ID").and_then(|v| v.as_str()).unwrap_or(""),
                "sql": sql_clean,
                "running_ms": row.get("QUERY_TIME_MS").and_then(|v| v.as_u64()),
                "user": row.get("USER").and_then(|v| v.as_str()).unwrap_or(""),
                "database": row.get("DATABASE").and_then(|v| v.as_str()).unwrap_or(""),
                "start_time": row.get("QUERY_START_TIME").and_then(|v| v.as_str()).unwrap_or(""),
                "workload_group_id": row.get("WORKLOAD_GROUP_ID").and_then(|v| v.as_u64()),
            })
        })
        .collect();

    Ok(Value::Array(queries))
}
