use std::{collections::HashSet, io};

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use super::{ProjectContent, ProjectItem, ProjectItemsEnvelope};

const MAX_ITEMS: usize = 10_000;
const MAX_PAGES: usize = 100;
const PAGE_SIZE: usize = 100;
const MAX_JSON_BYTES: usize = 16 * 1024 * 1024;

const RESOLVE_PROJECT: &str = r#"query($owner:String!,$number:Int!){
  repositoryOwner(login:$owner){
    ... on User{projectV2(number:$number){
      id number owner{... on User{login} ... on Organization{login}}
    }}
    ... on Organization{projectV2(number:$number){
      id number owner{... on User{login} ... on Organization{login}}
    }}
  }
  rateLimit{limit remaining cost resetAt}
}"#;

const PROJECT_ITEMS: &str = r#"query($id:ID!,$cursor:String){
  node(id:$id){
    ... on ProjectV2{
      id number owner{... on User{login} ... on Organization{login}}
      items(first:100,after:$cursor,archivedStates:[NOT_ARCHIVED]){
        totalCount pageInfo{endCursor hasNextPage}
        nodes{
          id isArchived
          fieldValueByName(name:"Status"){... on ProjectV2ItemFieldSingleSelectValue{name}}
          content{__typename ... on Issue{number title body url repository{nameWithOwner}}}
        }
      }
    }
  }
  rateLimit{limit remaining cost resetAt}
}"#;

/// Read a complete, non-archived Project without owning authentication, quota or persistence.
/// Opaque IDs/cursors are copied only from responses, never normalized or reconstructed.
pub(super) fn discover<F>(
    owner: &str,
    project_number: u32,
    mut fetch: F,
) -> Result<ProjectItemsEnvelope>
where
    F: FnMut(&str, Value) -> Result<Value>,
{
    ensure!(
        !owner.is_empty() && !owner.chars().any(char::is_whitespace),
        "GitHub Project owner must be nonempty and contain no whitespace"
    );
    ensure!(
        project_number > 0 && project_number <= i32::MAX as u32,
        "GitHub Project number must be a positive GraphQL Int"
    );

    let mut budget = JsonBudget(0);
    let response = fetch(
        RESOLVE_PROJECT,
        json!({"owner": owner, "number": project_number}),
    )
    .context("resolve GitHub Project")?;
    let data = response_data(&response, &mut budget)?;
    let project = data
        .pointer("/repositoryOwner/projectV2")
        .filter(|value| value.is_object())
        .context("GitHub Project or repository owner is unavailable")?;
    let project_id = project_identity(project, owner, project_number)?.to_owned();

    let mut items = Vec::new();
    let mut item_ids = HashSet::new();
    let mut cursors = HashSet::new();
    let mut cursor: Option<String> = None;
    let mut total_count = None;
    for page in 1..=MAX_PAGES {
        let response = fetch(PROJECT_ITEMS, json!({"id": project_id, "cursor": cursor}))
            .with_context(|| format!("fetch GitHub Project items page {page}"))?;
        let data = response_data(&response, &mut budget)?;
        let project = data
            .get("node")
            .filter(|value| value.is_object())
            .context("GitHub Project page node is null or missing")?;
        ensure!(
            project_identity(project, owner, project_number)? == project_id,
            "GitHub Project page changed its opaque Project ID"
        );
        let connection = project
            .get("items")
            .filter(|value| value.is_object())
            .context("GitHub Project items page is null or missing")?;
        let count = connection["totalCount"]
            .as_u64()
            .context("GitHub Project page omitted a valid totalCount")?;
        ensure!(
            count <= MAX_ITEMS as u64,
            "GitHub Project discovery exceeds the {MAX_ITEMS}-item limit"
        );
        let count = count as usize;
        ensure!(
            total_count.is_none_or(|previous| previous == count),
            "GitHub Project totalCount changed across pages"
        );
        total_count = Some(count);
        let nodes = connection["nodes"]
            .as_array()
            .context("GitHub Project page nodes are null or missing")?;
        ensure!(
            nodes.len() <= PAGE_SIZE,
            "GitHub Project page exceeds the {PAGE_SIZE}-item page size"
        );
        ensure!(
            items.len() + nodes.len() <= count,
            "GitHub Project returned more items than totalCount"
        );
        let page_info = connection
            .get("pageInfo")
            .context("GitHub Project pageInfo is missing")?;
        let has_next = page_info["hasNextPage"]
            .as_bool()
            .context("GitHub Project pageInfo omitted hasNextPage")?;
        ensure!(
            !nodes.is_empty() || !has_next,
            "GitHub Project pagination made no progress: empty page with hasNextPage"
        );
        let end_cursor = match page_info.get("endCursor") {
            Some(Value::Null) if nodes.is_empty() => None,
            Some(Value::String(value)) if !value.is_empty() && !nodes.is_empty() => {
                Some(value.as_str())
            }
            _ => bail!("GitHub Project page has a missing or invalid endCursor"),
        };
        if let Some(next) = end_cursor {
            ensure!(
                cursors.insert(next.to_owned()),
                "GitHub Project pagination repeated an endCursor (cursor loop or non-progress)"
            );
        }
        for node in nodes {
            let item = parse_item(node)
                .with_context(|| format!("decode GitHub Project item on page {page}"))?;
            ensure!(
                item_ids.insert(item.id.clone()),
                "GitHub Project discovery returned duplicate item IDs"
            );
            items.push(item);
        }
        if !has_next {
            ensure!(
                items.len() == count,
                "GitHub Project pagination truncated: received {} of {count} items",
                items.len()
            );
            return Ok(ProjectItemsEnvelope {
                items,
                total_count: count,
            });
        }
        ensure!(
            items.len() < count,
            "GitHub Project hasNextPage is true after reaching totalCount"
        );
        cursor = end_cursor.map(str::to_owned);
    }
    bail!("GitHub Project discovery exceeded the {MAX_PAGES}-page limit before completion")
}

fn project_identity<'a>(project: &'a Value, owner: &str, number: u32) -> Result<&'a str> {
    let id = string_field(project, "id")?;
    ensure!(!id.is_empty(), "GitHub Project ID is empty");
    ensure!(
        project["number"].as_u64() == Some(u64::from(number))
            && project
                .pointer("/owner/login")
                .and_then(Value::as_str)
                .is_some_and(|login| login.eq_ignore_ascii_case(owner)),
        "GitHub Project owner/number mismatch or missing identity"
    );
    Ok(id)
}

fn parse_item(node: &Value) -> Result<ProjectItem> {
    let id = string_field(node, "id")?;
    ensure!(!id.is_empty(), "GitHub Project item ID is empty");
    ensure!(
        node["isArchived"].as_bool() == Some(false),
        "GitHub Project returned an archived item or omitted isArchived"
    );
    let status = match node.get("fieldValueByName") {
        Some(Value::Null) => "",
        Some(Value::Object(field)) => match field.get("name") {
            Some(Value::String(name)) => name,
            // An inline fragment does not select a name for other field-value types.
            None => "",
            _ => bail!("GitHub Project item has an invalid Status name"),
        },
        _ => bail!("GitHub Project item omitted a valid Status field value"),
    };
    let content = node
        .get("content")
        .context("GitHub Project item omitted content")?;
    let content = if content.is_null() {
        ProjectContent::default()
    } else {
        let kind = string_field(content, "__typename")?;
        ensure!(!kind.is_empty(), "GitHub Project content type is empty");
        if kind == "Issue" {
            ProjectContent {
                kind: kind.to_owned(),
                number: content["number"]
                    .as_i64()
                    .filter(|number| *number > 0)
                    .context("GitHub Project Issue number is missing or invalid")?,
                title: string_field(content, "title")?.to_owned(),
                body: string_field(content, "body")?.to_owned(),
                url: string_field(content, "url")?.to_owned(),
                repository: string_field(&content["repository"], "nameWithOwner")?.to_owned(),
            }
        } else {
            // Keep every real item ID in the count, but never promote non-Issue content.
            ProjectContent {
                kind: kind.to_owned(),
                ..ProjectContent::default()
            }
        }
    };
    Ok(ProjectItem {
        id: id.to_owned(),
        status: status.to_owned(),
        content,
    })
}

fn string_field<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .with_context(|| format!("GitHub Project response omitted string field {field}"))
}

// Count all serialized response JSON, including resolution, metadata and placeholders, without
// allocating another potentially oversized body. Transport/raw-body limits belong to the caller.
struct JsonBudget(usize);

impl io::Write for JsonBudget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_JSON_BYTES - self.0 {
            return Err(io::Error::other("total JSON limit exceeded"));
        }
        self.0 += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn response_data<'a>(response: &'a Value, budget: &mut JsonBudget) -> Result<&'a Value> {
    serde_json::to_writer(budget, response)
        .context("GitHub Project discovery exceeded the 16 MiB total JSON limit")?;
    match response.get("errors") {
        None => {}
        Some(Value::Array(errors)) if errors.is_empty() => {}
        // Never accept GraphQL partial success or echo potentially sensitive remote error text.
        _ => bail!("GitHub Project discovery returned GraphQL errors or malformed errors"),
    }
    response
        .get("data")
        .filter(|data| data.is_object())
        .context("GitHub Project response data is null or missing")
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: &str = "Example";
    const NUMBER: u32 = 7;
    const PROJECT_ID: &str = "opaque-project:/A+b==";

    fn identity() -> Value {
        json!({
            "id": PROJECT_ID, "number": NUMBER,
            "owner": {"login": OWNER}
        })
    }

    fn response(data: Value) -> Value {
        let mut data = data;
        data["rateLimit"] = json!({
            "limit": 5000, "remaining": 4999, "cost": 1,
            "resetAt": "2026-09-06T12:00:00Z"
        });
        json!({"data": data})
    }

    fn resolved() -> Value {
        response(json!({"repositoryOwner": {"projectV2": identity()}}))
    }

    fn issue(number: usize) -> Value {
        json!({
            "id": format!("opaque-item-{number}"),
            "isArchived": false,
            "fieldValueByName": {"name": "Todo"},
            "content": {
                "__typename": "Issue", "number": number,
                "title": format!("Issue {number}"),
                "body": "  Exact source\r\nRésumé 🚀\n```rust\nfn main() {}\n```\n",
                "url": format!("https://github.com/Example/Repo/issues/{number}"),
                "repository": {"nameWithOwner": "Example/Repo"}
            }
        })
    }

    fn page(total: usize, nodes: Vec<Value>, cursor: Option<&str>, has_next: bool) -> Value {
        let mut project = identity();
        project["items"] = json!({
            "totalCount": total, "nodes": nodes,
            "pageInfo": {"endCursor": cursor, "hasNextPage": has_next}
        });
        response(json!({"node": project}))
    }

    fn discover_responses(responses: Vec<Value>) -> Result<ProjectItemsEnvelope> {
        let mut responses = responses.into_iter();
        discover(OWNER, NUMBER, |_, _| {
            Ok(responses.next().expect("unexpected extra GraphQL fetch"))
        })
    }

    fn discover_pages(pages: Vec<Value>) -> Result<ProjectItemsEnvelope> {
        discover_responses(std::iter::once(resolved()).chain(pages).collect())
    }

    fn failure(pages: Vec<Value>, expected: &str) {
        let error = format!("{:#}", discover_pages(pages).unwrap_err());
        assert!(error.contains(expected), "{error}");
    }

    #[test]
    fn two_pages_preserve_exact_source_and_opaque_variables() {
        let first_cursor = "opaque cursor:/C+d==";
        let last_cursor = "opaque cursor:/E+f==";
        let mut calls = 0;
        let result = discover("example", NUMBER, |query, variables| {
            calls += 1;
            let result = match calls {
                1 => {
                    assert_eq!(query, RESOLVE_PROJECT);
                    assert_eq!(variables, json!({"owner": "example", "number": NUMBER}));
                    resolved()
                }
                2 => {
                    assert_eq!(query, PROJECT_ITEMS);
                    assert_eq!(variables, json!({"id": PROJECT_ID, "cursor": null}));
                    page(
                        101,
                        (1..=100).map(issue).collect(),
                        Some(first_cursor),
                        true,
                    )
                }
                3 => {
                    assert_eq!(query, PROJECT_ITEMS);
                    assert_eq!(variables, json!({"id": PROJECT_ID, "cursor": first_cursor}));
                    page(101, vec![issue(101)], Some(last_cursor), false)
                }
                _ => panic!("discovery fetched after completion"),
            };
            Ok(result)
        })
        .unwrap();
        assert_eq!(calls, 3);
        assert_eq!(result.total_count, 101);
        assert_eq!(result.items.len(), 101);
        for (index, item) in result.items.iter().enumerate() {
            let source = issue(index + 1);
            assert_eq!(item.id, source["id"].as_str().unwrap());
            assert_eq!(item.status, "Todo");
            assert_eq!(item.content.kind, "Issue");
            assert_eq!(item.content.number, (index + 1) as i64);
            assert_eq!(
                item.content.title,
                source["content"]["title"].as_str().unwrap()
            );
            assert_eq!(
                item.content.body,
                source["content"]["body"].as_str().unwrap()
            );
            assert_eq!(item.content.url, source["content"]["url"].as_str().unwrap());
            assert_eq!(item.content.repository, "Example/Repo");
        }
    }

    #[test]
    fn queries_use_verified_fragments_filtered_items_and_rate_limit() {
        assert!(RESOLVE_PROJECT.contains("repositoryOwner(login:$owner)"));
        assert!(RESOLVE_PROJECT.contains("... on User{projectV2(number:$number)"));
        assert!(RESOLVE_PROJECT.contains("... on Organization{projectV2(number:$number)"));
        assert!(PROJECT_ITEMS.contains("node(id:$id)"));
        assert!(PROJECT_ITEMS.contains("... on ProjectV2{"));
        assert!(
            PROJECT_ITEMS.contains("items(first:100,after:$cursor,archivedStates:[NOT_ARCHIVED])")
        );
        assert!(PROJECT_ITEMS.contains("fieldValueByName(name:\"Status\")"));
        for query in [RESOLVE_PROJECT, PROJECT_ITEMS] {
            assert!(query.ends_with("  rateLimit{limit remaining cost resetAt}\n}"));
        }
    }

    #[test]
    fn supports_more_than_one_thousand_items_and_exact_maximum() {
        for total in [1002, MAX_ITEMS] {
            let mut calls = 0;
            let result = discover(OWNER, NUMBER, |query, variables| {
                calls += 1;
                if query == RESOLVE_PROJECT {
                    return Ok(resolved());
                }
                assert_eq!(query, PROJECT_ITEMS);
                let offset = (calls - 2) * PAGE_SIZE;
                let previous = (offset > 0).then(|| format!("cursor-{offset}"));
                assert_eq!(variables, json!({"id": PROJECT_ID, "cursor": previous}));
                let end = (offset + PAGE_SIZE).min(total);
                let nodes = (offset + 1..=end)
                    .map(|number| {
                        let mut node = issue(number);
                        if number < total {
                            node["content"] = Value::Null;
                        }
                        node
                    })
                    .collect();
                Ok(page(
                    total,
                    nodes,
                    Some(&format!("cursor-{end}")),
                    end < total,
                ))
            })
            .unwrap();
            assert_eq!(calls, 1 + total.div_ceil(PAGE_SIZE));
            assert_eq!(result.total_count, total);
            assert_eq!(result.items.len(), total);
            assert_eq!(result.items.last().unwrap().content.number, total as i64);
            assert_eq!(result.items.last().unwrap().content.kind, "Issue");
        }
    }

    #[test]
    fn accepts_a_genuinely_empty_project() {
        let result = discover_pages(vec![page(0, vec![], None, false)]).unwrap();
        assert_eq!(result.total_count, 0);
        assert!(result.items.is_empty());
    }

    #[test]
    fn user_and_organization_identity_match_case_insensitively() {
        for kind in ["User", "Organization"] {
            let mut resolution = resolved();
            resolution["data"]["repositoryOwner"]["__typename"] = json!(kind);
            resolution["data"]["repositoryOwner"]["projectV2"]["owner"]["__typename"] = json!(kind);
            let mut items_page = page(1, vec![issue(1)], Some("last"), false);
            items_page["data"]["node"]["owner"] =
                json!({"__typename": kind, "login": OWNER.to_ascii_uppercase()});
            let mut responses = [resolution, items_page].into_iter();
            let result = discover("example", NUMBER, |_, _| Ok(responses.next().unwrap())).unwrap();
            assert_eq!(result.items[0].content.repository, "Example/Repo");
            assert_eq!(result.items[0].content.number, 1);
        }
    }

    #[test]
    fn non_issue_and_null_content_remain_counted_non_candidates() {
        let nodes = ["PullRequest", "DraftIssue", ""]
            .into_iter()
            .enumerate()
            .map(|(index, kind)| {
                let mut node = issue(index + 1);
                node["content"] = if kind.is_empty() {
                    Value::Null
                } else {
                    json!({"__typename": kind, "number": 999, "title": "Not an Issue"})
                };
                node
            })
            .collect();
        let result = discover_pages(vec![page(3, nodes, Some("last"), false)]).unwrap();
        assert_eq!(result.total_count, 3);
        assert_eq!(result.items.len(), 3);
        for (index, kind) in ["PullRequest", "DraftIssue", ""].into_iter().enumerate() {
            let item = &result.items[index];
            assert_eq!(item.id, format!("opaque-item-{}", index + 1));
            assert_eq!(item.content.kind, kind);
            assert_eq!(item.content.number, 0);
            assert!(item.content.title.is_empty());
            assert!(item.content.body.is_empty());
            assert!(item.content.url.is_empty());
            assert!(item.content.repository.is_empty());
        }
    }

    #[test]
    fn unset_or_other_type_status_is_not_a_candidate_status() {
        for status in [Value::Null, json!({})] {
            let mut node = issue(1);
            node["fieldValueByName"] = status;
            let result = discover_pages(vec![page(1, vec![node], Some("last"), false)]).unwrap();
            assert!(result.items[0].status.is_empty());
        }
    }

    #[test]
    fn rejects_archived_missing_identity_and_malformed_source_rows() {
        for (path, value, expected) in [
            ("/id", Value::Null, "string field id"),
            ("/id", json!(""), "item ID is empty"),
            ("/isArchived", json!(true), "archived"),
            ("/isArchived", Value::Null, "isArchived"),
            ("/content/__typename", Value::Null, "__typename"),
            ("/content/__typename", json!(""), "content type is empty"),
            ("/content/number", Value::Null, "Issue number"),
            ("/content/number", json!(0), "Issue number"),
            ("/content/title", Value::Null, "title"),
            ("/content/body", Value::Null, "body"),
            ("/content/url", Value::Null, "url"),
            (
                "/content/repository/nameWithOwner",
                Value::Null,
                "nameWithOwner",
            ),
            ("/fieldValueByName", json!("Todo"), "Status field"),
            ("/fieldValueByName/name", json!(17), "Status name"),
        ] {
            let mut node = issue(1);
            *node.pointer_mut(path).unwrap() = value;
            failure(vec![page(1, vec![node], Some("last"), false)], expected);
        }
        for field in ["id", "isArchived", "content", "fieldValueByName"] {
            let mut node = issue(1);
            node.as_object_mut().unwrap().remove(field);
            assert!(discover_pages(vec![page(1, vec![node], Some("last"), false)]).is_err());
        }
        failure(vec![page(1, vec![Value::Null], Some("last"), false)], "id");
    }

    #[test]
    fn rejects_project_identity_mismatch_at_resolution_and_on_later_pages() {
        for (field, value) in [
            ("id", Value::Null),
            ("id", json!("")),
            ("number", json!(NUMBER + 1)),
            ("owner", json!({"login": "another-owner"})),
            ("owner", Value::Null),
        ] {
            let mut resolution = resolved();
            resolution["data"]["repositoryOwner"]["projectV2"][field] = value;
            assert!(discover_responses(vec![resolution]).is_err());
        }
        for (field, value) in [
            ("id", json!("different-opaque-project")),
            ("id", Value::Null),
            ("number", json!(NUMBER + 1)),
            ("owner", json!({"login": "another-owner"})),
        ] {
            let first = page(2, vec![issue(1)], Some("first"), true);
            let mut second = page(2, vec![issue(2)], Some("last"), false);
            second["data"]["node"][field] = value;
            assert!(discover_pages(vec![first, second]).is_err());
        }
    }

    #[test]
    fn rejects_null_or_missing_resolution_data() {
        for path in [
            "/data",
            "/data/repositoryOwner",
            "/data/repositoryOwner/projectV2",
        ] {
            let mut resolution = resolved();
            *resolution.pointer_mut(path).unwrap() = Value::Null;
            assert!(discover_responses(vec![resolution]).is_err(), "{path}");
        }
        for resolution in [Value::Null, json!({}), json!({"data": {}})] {
            assert!(discover_responses(vec![resolution]).is_err());
        }
    }

    #[test]
    fn rejects_null_or_missing_pages_and_pagination_metadata() {
        for path in [
            "/data",
            "/data/node",
            "/data/node/items",
            "/data/node/items/nodes",
            "/data/node/items/totalCount",
            "/data/node/items/pageInfo",
            "/data/node/items/pageInfo/hasNextPage",
        ] {
            let mut invalid = page(2, vec![issue(2)], Some("last"), false);
            *invalid.pointer_mut(path).unwrap() = Value::Null;
            assert!(
                discover_pages(vec![page(2, vec![issue(1)], Some("first"), true), invalid])
                    .is_err(),
                "{path}"
            );
        }
        for field in ["totalCount", "nodes", "pageInfo"] {
            let mut invalid = page(0, vec![], None, false);
            invalid["data"]["node"]["items"]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(discover_pages(vec![invalid]).is_err(), "{field}");
        }
    }

    #[test]
    fn rejects_partial_graphql_errors_at_every_stage_without_echoing_them() {
        for stage in 0..3 {
            for errors in [
                json!([{"message": "private remote detail"}]),
                json!({"message": "private remote detail"}),
                Value::Null,
            ] {
                let mut responses = vec![
                    resolved(),
                    page(2, vec![issue(1)], Some("first"), true),
                    page(2, vec![issue(2)], Some("last"), false),
                ];
                responses[stage]["errors"] = errors;
                let error = format!("{:#}", discover_responses(responses).unwrap_err());
                assert!(error.contains("GraphQL errors"), "{error}");
                assert!(!error.contains("private remote detail"));
            }
        }
    }

    #[test]
    fn accepts_an_empty_errors_array() {
        let mut resolution = resolved();
        resolution["errors"] = json!([]);
        let mut items_page = page(0, vec![], None, false);
        items_page["errors"] = json!([]);
        assert!(discover_responses(vec![resolution, items_page]).is_ok());
    }

    #[test]
    fn propagates_fetch_failure_without_returning_accumulated_items() {
        for fail_on_call in 1..=3 {
            let mut calls = 0;
            let result = discover(OWNER, NUMBER, |_, _| {
                calls += 1;
                if calls == fail_on_call {
                    bail!("transport quota refusal");
                }
                Ok(if calls == 1 {
                    resolved()
                } else {
                    page(2, vec![issue(1)], Some("first"), true)
                })
            });
            assert!(format!("{:#}", result.unwrap_err()).contains("transport quota refusal"));
            assert_eq!(calls, fail_on_call);
        }
    }

    #[test]
    fn rejects_missing_null_empty_or_wrong_type_cursors_even_on_final_pages() {
        for has_next in [true, false] {
            for cursor in [Value::Null, json!(""), json!(7)] {
                let mut invalid = page(2, vec![issue(1)], Some("first"), has_next);
                invalid["data"]["node"]["items"]["pageInfo"]["endCursor"] = cursor;
                failure(vec![invalid], "endCursor");
            }
            let mut invalid = page(2, vec![issue(1)], Some("first"), has_next);
            invalid["data"]["node"]["items"]["pageInfo"]
                .as_object_mut()
                .unwrap()
                .remove("endCursor");
            failure(vec![invalid], "endCursor");
        }
    }

    #[test]
    fn rejects_repeated_and_looping_cursors_including_the_terminal_page() {
        for last_has_next in [true, false] {
            failure(
                vec![
                    page(3, vec![issue(1)], Some("first"), true),
                    page(3, vec![issue(2)], Some("first"), last_has_next),
                ],
                "repeated an endCursor",
            );
        }
        failure(
            vec![
                page(4, vec![issue(1)], Some("a"), true),
                page(4, vec![issue(2)], Some("b"), true),
                page(4, vec![issue(3)], Some("a"), true),
            ],
            "cursor loop",
        );
    }

    #[test]
    fn rejects_empty_non_progress_and_early_truncation() {
        failure(vec![page(1, vec![], None, true)], "no progress");
        failure(vec![page(1, vec![], None, false)], "truncated");
        failure(
            vec![
                page(3, vec![issue(1)], Some("first"), true),
                page(3, vec![issue(2)], Some("last"), false),
            ],
            "received 2 of 3",
        );
        failure(
            vec![page(1, vec![issue(1)], Some("first"), true)],
            "after reaching totalCount",
        );
        failure(
            vec![page(0, vec![issue(1)], Some("last"), false)],
            "more items than totalCount",
        );
    }

    #[test]
    fn rejects_duplicate_item_ids_within_and_across_pages_even_for_placeholders() {
        let mut placeholder = issue(1);
        placeholder["content"] = Value::Null;
        failure(
            vec![page(
                2,
                vec![issue(1), placeholder.clone()],
                Some("last"),
                false,
            )],
            "duplicate item IDs",
        );
        failure(
            vec![
                page(2, vec![placeholder], Some("first"), true),
                page(2, vec![issue(1)], Some("last"), false),
            ],
            "duplicate item IDs",
        );
    }

    #[test]
    fn rejects_total_count_drift_in_both_directions() {
        for changed_count in [1, 3] {
            failure(
                vec![
                    page(2, vec![issue(1)], Some("first"), true),
                    page(changed_count, vec![issue(2)], Some("last"), false),
                ],
                "totalCount changed",
            );
        }
    }

    #[test]
    fn enforces_item_and_page_size_limits() {
        failure(
            vec![page(MAX_ITEMS + 1, vec![], None, false)],
            "10000-item limit",
        );
        failure(
            vec![page(
                101,
                (1..=101).map(issue).collect(),
                Some("last"),
                false,
            )],
            "100-item page size",
        );
    }

    #[test]
    fn enforces_page_limit_without_fetching_page_one_hundred_and_one() {
        let mut calls = 0;
        let result = discover(OWNER, NUMBER, |_, _| {
            calls += 1;
            if calls == 1 {
                return Ok(resolved());
            }
            let index = calls - 1;
            assert!(index <= MAX_PAGES, "fetched beyond the page budget");
            Ok(page(
                101,
                vec![issue(index)],
                Some(&format!("cursor-{index}")),
                true,
            ))
        });
        assert!(format!("{:#}", result.unwrap_err()).contains("100-page limit"));
        assert_eq!(calls, MAX_PAGES + 1);
    }

    #[test]
    fn enforces_total_json_budget_including_resolution_and_ignored_fields() {
        let mut resolution = resolved();
        resolution["padding"] = json!("x".repeat(MAX_JSON_BYTES));
        let error = format!("{:#}", discover_responses(vec![resolution]).unwrap_err());
        assert!(error.contains("16 MiB total JSON limit"), "{error}");

        let mut first = issue(1);
        first["content"]["body"] = json!("x".repeat(8 * 1024 * 1024));
        let mut second = issue(2);
        second["content"]["body"] = json!("x".repeat(8 * 1024 * 1024));
        failure(
            vec![
                page(2, vec![first], Some("first"), true),
                page(2, vec![second], Some("last"), false),
            ],
            "16 MiB total JSON limit",
        );
    }

    #[test]
    fn json_budget_counts_serialized_utf8_and_escaping_at_the_exact_boundary() {
        let value = json!({"text": "🚀\n\"é"});
        let length = serde_json::to_vec(&value).unwrap().len();
        let mut exact = JsonBudget(MAX_JSON_BYTES - length);
        serde_json::to_writer(&mut exact, &value).unwrap();
        assert_eq!(exact.0, MAX_JSON_BYTES);
        let mut over = JsonBudget(MAX_JSON_BYTES - length + 1);
        assert!(serde_json::to_writer(&mut over, &value).is_err());
    }

    #[test]
    fn invalid_requested_identity_never_calls_fetch() {
        for (owner, number) in [
            ("", NUMBER),
            ("example ", NUMBER),
            (OWNER, 0),
            (OWNER, i32::MAX as u32 + 1),
        ] {
            assert!(
                discover(owner, number, |_, _| panic!(
                    "invalid request fetched GitHub"
                ))
                .is_err()
            );
        }
    }
}
