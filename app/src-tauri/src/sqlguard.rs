//! Lexing helpers for the read-only guard, prod write confirmation and GO batch splitting.

/// Upper-cased bare words of a T-SQL batch. Comments, string literals and
/// quoted/bracketed identifiers are skipped, so `[Update]` or `'drop'` never match.
pub fn words(sql: &str) -> Vec<String> {
    let c: Vec<char> = sql.chars().collect();
    let (mut i, mut out) = (0, Vec::new());
    while i < c.len() {
        let ch = c[i];
        if ch == '-' && c.get(i + 1) == Some(&'-') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch == '/' && c.get(i + 1) == Some(&'*') {
            // T-SQL block comments nest.
            let mut depth = 0;
            while i < c.len() {
                if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else if ch == '\'' || ch == '"' || ch == '[' {
            let close = if ch == '[' { ']' } else { ch };
            i += 1;
            while i < c.len() {
                if c[i] == close {
                    if c.get(i + 1) == Some(&close) {
                        i += 2; // escaped '' "" ]]
                        continue;
                    }
                    break;
                }
                i += 1;
            }
            i += 1;
        } else if ch.is_alphanumeric() || ch == '_' || ch == '@' || ch == '#' {
            let start = i;
            while i < c.len() && (c[i].is_alphanumeric() || matches!(c[i], '_' | '@' | '#' | '$')) {
                i += 1;
            }
            out.push(c[start..i].iter().collect::<String>().to_uppercase());
        } else {
            i += 1;
        }
    }
    out
}

const WRITE_WORDS: &[&str] = &[
    "INSERT", "UPDATE", "DELETE", "MERGE", "CREATE", "ALTER", "DROP", "TRUNCATE", "EXEC", "EXECUTE",
    "GRANT", "REVOKE", "DENY", "INTO", "COMMIT", "ROLLBACK", "SAVE", "TRAN", "TRANSACTION", "BACKUP",
    "RESTORE", "DBCC", "KILL", "SHUTDOWN", "RECONFIGURE", "CHECKPOINT", "BULK", "OPENROWSET",
    "OPENQUERY", "OPENDATASOURCE", "WRITETEXT", "UPDATETEXT", "ENABLE", "DISABLE", "RECEIVE", "SEND",
    "MOVE", "SETUSER", "REVERT", "SP_EXECUTESQL",
];
// A batch whose first word is anything else may be a bare procedure call (`sp_foo 1`).
const READ_START: &[&str] = &["SELECT", "WITH", "DECLARE", "SET", "PRINT", "IF", "BEGIN", "WHILE", "USE"];

/// None when the batch is read-only, else the reason it is not.
/// shortcut: keyword deny-list, fails closed (a bare column named e.g. `Merge` is blocked); the
/// backend also wraps read-only runs in a rolled-back transaction as a second layer.
pub fn write_reason(sql: &str) -> Option<String> {
    let w = words(sql);
    if let Some(first) = w.first() {
        if !READ_START.contains(&first.as_str()) {
            return Some(format!("statement starts with {first}"));
        }
    }
    w.iter()
        .find(|x| {
            WRITE_WORDS.contains(&x.as_str())
                // SQL Server splits `1DELETE` / `1E0DELETE` into a number and a keyword; fail closed.
                || (x.starts_with(|c: char| c.is_ascii_digit()) && WRITE_WORDS.iter().any(|k| x.contains(k)))
        })
        .map(|x| format!("contains {x}"))
}

/// True when the batch has DML whose row count we can report by appending `SELECT @@ROWCOUNT`.
/// Never for batches with CREATE: a procedure/view/trigger body runs to the end of the batch.
pub fn wants_rowcount(sql: &str) -> bool {
    let w = words(sql);
    !w.iter().any(|x| x == "CREATE" || x == "ALTER")
        && w.iter().any(|x| matches!(x.as_str(), "INSERT" | "UPDATE" | "DELETE" | "MERGE"))
}

/// Split a script on lines that contain only `GO` (SSMS batch separator).
/// shortcut: ignores `GO n` repeat counts and a GO line inside a multi-line string.
pub fn split_batches(sql: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    for line in sql.lines() {
        if line.trim().eq_ignore_ascii_case("go") {
            out.push(String::new());
        } else {
            let cur = out.last_mut().unwrap();
            cur.push_str(line);
            cur.push('\n');
        }
    }
    out.retain(|b| !words(b).is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_guard() {
        for ok in [
            "SELECT * FROM t",
            "select [Update], 'drop table x' from t -- delete\n/* insert /* nested */ exec */",
            "WITH c AS (SELECT 1 a) SELECT * FROM c",
            "DECLARE @x int = 1; SELECT @x",
            "SELECT \"Insert\" FROM t",
        ] {
            assert_eq!(write_reason(ok), None, "{ok}");
        }
        for bad in [
            "UPDATE t SET a=1",
            "SELECT * INTO x FROM t",
            "select 1; delete from t",
            "sp_who",
            "EXEC sp_who",
            "SELECT 1; COMMIT",
            "/* x */ drop table t",
            "SELECT 'it''s'; truncate table t",
            "SELECT [a]]b]; delete t",
            "SELECT 1DELETE FROM t SELECT 1COMMIT",
            "SELECT 1E0DELETE FROM t",
        ] {
            assert!(write_reason(bad).is_some(), "{bad}");
        }
    }

    #[test]
    fn batches_and_rowcount() {
        assert_eq!(split_batches("select 1\nGO\n  go  \nselect 2\n-- only comment\nGo").len(), 2);
        assert!(wants_rowcount("update t set a=1"));
        assert!(!wants_rowcount("create proc p as update t set a=1"));
        assert!(!wants_rowcount("select [update] from t"));
    }
}
