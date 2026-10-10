//! One level of a tree (E38, rules T1–T6, plan/spezifikation/17-baum.md).
//!
//! The hierarchy is the whole table: a node's parent is the row whose key
//! equals its parent field (T1). The filter runs over every row; what is shown
//! are the matches **and their ancestors**, the ancestors that do not match as
//! context (T5). Of that, the query asks for one level — the children of
//! `under`, or the roots (T2, T4) — sorted among themselves and paged (T6).
//! Asked for, each node of the level carries aggregates over its subtree (T7).
//!
//! Keys are compared by equality only (S7, as the pivot's assembly does), and
//! the order is [`opengrid_columns::sort::order`]'s, so a tree adds no second
//! way of comparing or ordering values.

use std::collections::HashMap;

use opengrid_columns::Table;
use opengrid_columns::sort::{SortKey, order};
use opengrid_datasource::{FlatTree, TreeLevel};
use opengrid_query::{NullsOrder, SortDirection, ValidatedQuery, ValidatedTree};
use opengrid_types::Value;

use super::{ExecuteError, QueryResult, aggregate, column, filter, project};

/// A key as something to look up: equal keys are equal strings (S7 — `NaN`
/// is `NaN`, `-0.0` is `0.0`). NULL is no key.
fn identity(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Float64(number) if number.is_nan() => Some("nan".to_owned()),
        Value::Float64(number) if *number == 0.0 => Some("0".to_owned()),
        other => Some(opengrid_json::to_string(other)),
    }
}

/// The key as a sentence names it.
fn named(value: &Value) -> String {
    match value {
        Value::Utf8(text) => format!("{text:?}"),
        other => opengrid_json::to_string(other),
    }
}

/// Runs `query`, whose tree part is `tree`, against `table`.
pub(crate) fn execute(
    table: &Table,
    query: &ValidatedQuery,
    tree: &ValidatedTree,
) -> Result<QueryResult, ExecuteError> {
    // The scope first (plan point 122): a row outside it is not part of the
    // tree at all, so it can be neither context nor a parent.
    let scoped;
    let table = match &tree.scope {
        None => table,
        Some(scope) => {
            let keep: Vec<u32> = filter::evaluate(scope, table)?
                .iter()
                .enumerate()
                .filter(|(_, hit)| **hit == Some(true))
                .map(|(row, _)| row as u32)
                .collect();
            scoped = table.take(&keep);
            &scoped
        }
    };
    let keys = column(table, tree.key.as_str())?;
    let parents = column(table, tree.parent.as_str())?;
    let rows = table.num_rows();

    // T1: every key once.
    let mut by_key: HashMap<String, u32> = HashMap::with_capacity(rows);
    for row in 0..rows {
        if let Some(id) = identity(&keys.value(row))
            && by_key.insert(id, row as u32).is_some()
        {
            return Err(ExecuteError::Tree {
                message: format!(
                    "tree: the key {} appears more than once (rule T1)",
                    named(&keys.value(row))
                ),
            });
        }
    }

    // T2: a parent that is NULL or names no node makes a root; the second kind
    // is an orphan, and the answer counts them.
    let mut parent_of: Vec<Option<u32>> = Vec::with_capacity(rows);
    let mut orphans = 0u64;
    for row in 0..rows {
        match identity(&parents.value(row)) {
            None => parent_of.push(None),
            Some(id) => match by_key.get(&id) {
                Some(parent) => parent_of.push(Some(*parent)),
                None => {
                    orphans += 1;
                    parent_of.push(None);
                }
            },
        }
    }

    // T3: following the parents from any node reaches a root. A walk that
    // meets itself is a cycle, named by one of its nodes. Each node is walked
    // once (`Done`), so the check is linear.
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        New,
        Walking,
        Done,
    }
    let mut mark = vec![Mark::New; rows];
    let mut path: Vec<usize> = Vec::new();
    for start in 0..rows {
        let mut at = start;
        while mark[at] == Mark::New {
            mark[at] = Mark::Walking;
            path.push(at);
            match parent_of[at] {
                Some(parent) => at = parent as usize,
                None => break,
            }
        }
        if mark[at] == Mark::Walking && parent_of[at].is_some() && path.contains(&at) {
            return Err(ExecuteError::Tree {
                message: format!(
                    "tree: the parents of {} lead back to it — a cycle (rule T3)",
                    named(&keys.value(at))
                ),
            });
        }
        for node in path.drain(..) {
            mark[node] = Mark::Done;
        }
    }

    // T5: the matches, and every ancestor of one.
    let matched: Vec<bool> = match &query.filter {
        Some(expression) => filter::evaluate(expression, table)?
            .iter()
            .map(|hit| *hit == Some(true))
            .collect(),
        None => vec![true; rows],
    };
    let matches = matched.iter().filter(|hit| **hit).count() as u64;
    let mut visible = vec![false; rows];
    for row in (0..rows).filter(|row| matched[*row]) {
        let mut at = Some(row as u32);
        while let Some(node) = at {
            if visible[node as usize] {
                break;
            }
            visible[node as usize] = true;
            at = parent_of[node as usize];
        }
    }
    let mut children = vec![0u64; rows];
    for row in (0..rows).filter(|row| visible[*row]) {
        if let Some(parent) = parent_of[row] {
            children[parent as usize] += 1;
        }
    }
    // T7: who hangs from whom, for the subtree aggregates — built only when
    // asked for. Every match is visible, so the visible nodes are enough.
    let kids: Option<Vec<Vec<u32>>> = (!tree.aggregate.is_empty()).then(|| {
        let mut kids = vec![Vec::new(); rows];
        for row in (0..rows).filter(|row| visible[*row]) {
            if let Some(parent) = parent_of[row] {
                kids[parent as usize].push(row as u32);
            }
        }
        kids
    });
    let walk = Subtrees {
        kids: kids.as_deref(),
        matched: &matched,
    };

    // T8: the whole tree, flat — no level to pick.
    if tree.flat {
        return flat(
            table, query, tree, &parent_of, &visible, &children, &matched, matches, orphans,
        );
    }

    // T4: the level — the visible children of `under`, or the visible roots.
    let under = match &tree.under {
        None => None,
        Some(value) => match identity(value).and_then(|id| by_key.get(&id)) {
            Some(row) => Some(*row),
            // A node that does not exist has no children.
            None => {
                return level(
                    table,
                    query,
                    tree,
                    Vec::new(),
                    &children,
                    &walk,
                    matches,
                    orphans,
                );
            }
        },
    };
    let members: Vec<u32> = (0..rows as u32)
        .filter(|row| visible[*row as usize] && parent_of[*row as usize] == under)
        .collect();
    level(
        table, query, tree, members, &children, &walk, matches, orphans,
    )
}

/// The whole visible tree, flat (T8, issue #166): depth-first, each node's
/// children in the query's order among siblings (T6, ties by the key), then
/// paged like any result. Each row carries its level and its path of keys.
#[allow(clippy::too_many_arguments)]
fn flat(
    table: &Table,
    query: &ValidatedQuery,
    tree: &ValidatedTree,
    parent_of: &[Option<u32>],
    visible: &[bool],
    children: &[u64],
    matched: &[bool],
    matches: u64,
    orphans: u64,
) -> Result<QueryResult, ExecuteError> {
    let shown: Vec<u32> = (0..table.num_rows() as u32)
        .filter(|row| visible[*row as usize])
        .collect();
    let nodes = table.take(&shown);

    // One order over every shown node; siblings keep it among themselves.
    let mut sort_keys = Vec::with_capacity(query.sort.len() + 1);
    for key in &query.sort {
        sort_keys.push(SortKey {
            column: column(&nodes, key.field.as_str())?,
            descending: matches!(key.direction, SortDirection::Desc),
            nulls_first: matches!(key.nulls, NullsOrder::First),
        });
    }
    let key_column = column(&nodes, tree.key.as_str())?;
    sort_keys.push(SortKey {
        column: key_column,
        descending: false,
        nulls_first: false,
    });
    let ranked =
        order(&sort_keys, shown.len()).map_err(|message| ExecuteError::TooLarge { message })?;

    // Children and roots, in that order: walking the ranking once fills them.
    let mut kids: Vec<Vec<u32>> = vec![Vec::new(); table.num_rows()];
    let mut roots = Vec::new();
    for at in &ranked {
        let row = shown[*at as usize];
        match parent_of[row as usize] {
            Some(parent) => kids[parent as usize].push(row),
            None => roots.push(row),
        }
    }

    // Depth-first; the path is the keys from the root down.
    let keys = column(table, tree.key.as_str())?;
    let mut sequence: Vec<(u32, u64, Vec<Value>)> = Vec::with_capacity(shown.len());
    let mut stack: Vec<(u32, u64, Vec<Value>)> = roots
        .iter()
        .rev()
        .map(|root| (*root, 1, vec![keys.value(*root as usize)]))
        .collect();
    while let Some((row, level, path)) = stack.pop() {
        for child in kids[row as usize].iter().rev() {
            let mut below = path.clone();
            below.push(keys.value(*child as usize));
            stack.push((*child, level + 1, below));
        }
        sequence.push((row, level, path));
    }

    let count = sequence.len();
    let offset = usize::try_from(query.offset.unwrap_or(0))
        .unwrap_or(usize::MAX)
        .min(count);
    let end = query.limit.map_or(count, |limit| {
        offset
            .saturating_add(usize::try_from(limit).unwrap_or(usize::MAX))
            .min(count)
    });
    let page = &sequence[offset..end];
    let rows: Vec<u32> = page.iter().map(|(row, _, _)| *row).collect();
    Ok(QueryResult {
        table: project(&table.take(&rows), &query.output_schema)?,
        total_count: count as u64,
        tree: Some(TreeLevel {
            children: rows.iter().map(|row| children[*row as usize]).collect(),
            matched: rows.iter().map(|row| matched[*row as usize]).collect(),
            matches,
            orphans,
            flat: Some(FlatTree {
                levels: page.iter().map(|(_, level, _)| *level).collect(),
                paths: page.iter().map(|(_, _, path)| path.clone()).collect(),
                key_type: keys.data_type(),
                filtered: query.filter.is_some(),
            }),
            ..TreeLevel::default()
        }),
    })
}

/// The subtrees of a tree, for the aggregates of T7.
struct Subtrees<'a> {
    /// The visible children of each node; `None` when no aggregate was asked for.
    kids: Option<&'a [Vec<u32>]>,
    matched: &'a [bool],
}

impl Subtrees<'_> {
    /// The matches among `node` and its descendants — context does not count
    /// (T7), whatever it holds.
    fn matches_under(&self, node: u32, kids: &[Vec<u32>]) -> Vec<u32> {
        let mut found = Vec::new();
        let mut stack = vec![node];
        while let Some(at) = stack.pop() {
            if self.matched[at as usize] {
                found.push(at);
            }
            stack.extend_from_slice(&kids[at as usize]);
        }
        found
    }
}

/// Sorts and pages one level (T6) and says what each of its rows is.
#[allow(clippy::too_many_arguments)]
fn level(
    table: &Table,
    query: &ValidatedQuery,
    tree: &ValidatedTree,
    members: Vec<u32>,
    children: &[u64],
    walk: &Subtrees<'_>,
    matches: u64,
    orphans: u64,
) -> Result<QueryResult, ExecuteError> {
    let siblings = table.take(&members);
    // T6: the query's keys among siblings, ties by the key ascending — the
    // order is total, so a page is always the same page.
    let mut sort_keys = Vec::with_capacity(query.sort.len() + 1);
    for key in &query.sort {
        sort_keys.push(SortKey {
            column: column(&siblings, key.field.as_str())?,
            descending: matches!(key.direction, SortDirection::Desc),
            nulls_first: matches!(key.nulls, NullsOrder::First),
        });
    }
    sort_keys.push(SortKey {
        column: column(&siblings, tree.key.as_str())?,
        descending: false,
        nulls_first: false,
    });
    let count = siblings.num_rows();
    let offset = usize::try_from(query.offset.unwrap_or(0))
        .unwrap_or(usize::MAX)
        .min(count);
    let end = query.limit.map_or(count, |limit| {
        offset
            .saturating_add(usize::try_from(limit).unwrap_or(usize::MAX))
            .min(count)
    });
    let positions = order(&sort_keys, end).map_err(|message| ExecuteError::TooLarge { message })?;
    let page = &positions[offset..];

    let rows: Vec<usize> = page
        .iter()
        .map(|at| members[*at as usize] as usize)
        .collect();
    // T7: per node of the page, over the matches of its subtree. Siblings'
    // subtrees are disjoint, so a level costs one pass over the tree at most.
    let aggregates = match walk.kids {
        None => Vec::new(),
        Some(kids) => {
            let groups: Vec<Vec<u32>> = rows
                .iter()
                .map(|row| walk.matches_under(*row as u32, kids))
                .collect();
            aggregate::over_groups(table, &tree.aggregate, &tree.aggregate_schema, &groups)?
                .iter()
                .map(|column| (0..column.len()).map(|row| column.value(row)).collect())
                .collect()
        }
    };
    Ok(QueryResult {
        table: project(&siblings.take(page), &query.output_schema)?,
        total_count: count as u64,
        tree: Some(TreeLevel {
            children: rows.iter().map(|row| children[*row]).collect(),
            matched: rows.iter().map(|row| walk.matched[*row]).collect(),
            matches,
            orphans,
            aggregate_schema: if aggregates.is_empty() {
                Default::default()
            } else {
                tree.aggregate_schema.clone()
            },
            aggregates,
            flat: None,
        }),
    })
}
