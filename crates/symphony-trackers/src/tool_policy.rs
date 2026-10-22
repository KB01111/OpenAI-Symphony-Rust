//! What an agent may do through the host-side `github_api` and `linear_graphql` tools.
//!
//! The tools run with Symphony's own tracker credentials, outside the agent's sandbox and outside
//! the approval gate, and an agent reads untrusted text (issue bodies, comments, files) that can
//! carry instructions. Without a policy a prompt-injected agent could merge pull requests, delete
//! issues or reach other repositories the token can see. The policy is an allowlist: it keeps the
//! tools useful for what an implementation run needs (read the project, comment, open and update
//! issues, update pull requests of the project's own repository) and refuses everything else,
//! including all merges. Merging stays a human decision.

/// Check a `github_api` request against the policy of the project's repository (`owner/name`).
pub fn check_github_request(repo: &str, method: &str, path: &str) -> Result<(), String> {
    let method = method.trim().to_ascii_uppercase();
    if !matches!(method.as_str(), "GET" | "POST" | "PATCH") {
        return Err(format!(
            "github_api does not allow {method}: only GET, POST and PATCH are available, so merges, deletions and replacements are not possible"
        ));
    }
    if !path.starts_with('/') || path.starts_with("//") || path.contains("://") {
        return Err("github_api path must be a relative API path starting with '/'".into());
    }
    if path.contains('\\') || path.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("github_api path contains characters that are not allowed".into());
    }
    let lowered = path.to_ascii_lowercase();
    if lowered.contains("%2e") || lowered.contains("%2f") || lowered.contains("%5c") {
        return Err("github_api path must not use encoded dots or separators".into());
    }
    let route = path.split(['?', '#']).next().unwrap_or(path);
    let segments: Vec<&str> = route.trim_matches('/').split('/').collect();
    if segments
        .iter()
        .any(|s| s.is_empty() || *s == "." || *s == "..")
    {
        return Err("github_api path must not contain empty or relative segments".into());
    }

    let (owner, name) = repo.trim_matches('/').split_once('/').ok_or_else(|| {
        "the project's GitHub repository is not configured as owner/name".to_string()
    })?;
    match segments.as_slice() {
        ["repos", o, n, rest @ ..] if o.eq_ignore_ascii_case(owner) && n.eq_ignore_ascii_case(name) => {
            check_repo_route(&method, rest)
        }
        _ => Err(format!(
            "github_api is limited to the project's own repository: paths must start with /repos/{owner}/{name}/"
        )),
    }
}

fn check_repo_route(method: &str, rest: &[&str]) -> Result<(), String> {
    let deny = || {
        Err(format!(
            "github_api does not allow {method} /repos/…/{}: it is limited to reading the repository and working with its issues, comments and pull requests",
            rest.join("/")
        ))
    };
    let is_number = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    match (method, rest) {
        ("GET", [first, tail @ ..]) => {
            const READABLE: &[&str] = &[
                "issues",
                "pulls",
                "commits",
                "contents",
                "branches",
                "compare",
                "labels",
                "milestones",
                "readme",
                "languages",
                "releases",
                "tags",
                "check-runs",
                "check-suites",
                "statuses",
                "git",
            ];
            let allowed = READABLE.contains(first)
                // Workflow runs and jobs only: never secrets, variables or caches.
                || (*first == "actions" && matches!(tail.first(), Some(&"runs" | &"jobs" | &"workflows")));
            if allowed {
                Ok(())
            } else {
                deny()
            }
        }
        // Create an issue. Pull requests are opened only by the explicit "Create pull request"
        // action, which commits, pushes and asks for confirmation; an agent never opens one here.
        ("POST", ["issues"]) => Ok(()),
        // Comment on / label an issue or pull request.
        ("POST", ["issues", n, "comments" | "labels"]) if is_number(n) => Ok(()),
        // Review comments on a pull request diff (not reviews: an approving review is a decision).
        ("POST", ["pulls", n, "comments"]) if is_number(n) => Ok(()),
        // Edit an issue or a pull request, or one of their comments.
        ("PATCH", ["issues" | "pulls", n]) if is_number(n) => Ok(()),
        ("PATCH", ["issues" | "pulls", "comments", id]) if is_number(id) => Ok(()),
        _ => deny(),
    }
}

/// Mutations the `linear_graphql` tool may run: commenting, updating issues and linking pull
/// requests. Queries are always read-only and always allowed.
const LINEAR_ALLOWED_MUTATIONS: &[&str] = &[
    "commentCreate",
    "commentUpdate",
    "issueUpdate",
    "attachmentCreate",
    "attachmentLinkGitHubPR",
    "attachmentLinkURL",
];

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Name(String),
    Punct(char),
    Spread,
}

/// GraphQL lexer, enough for the policy: comments, strings (block strings included) and numbers are
/// dropped, so nothing inside them can be mistaken for a keyword or a field.
fn tokenize(source: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() || c == ',' || c == '\u{feff}' => i += 1,
            '#' => {
                while i < chars.len() && chars[i] != '\n' && chars[i] != '\r' {
                    i += 1;
                }
            }
            '"' => {
                if chars[i..].starts_with(&['"', '"', '"']) {
                    i += 3;
                    loop {
                        if i >= chars.len() {
                            return Err("unterminated block string".into());
                        }
                        if chars[i..].starts_with(&['\\', '"', '"', '"']) {
                            i += 4;
                        } else if chars[i..].starts_with(&['"', '"', '"']) {
                            i += 3;
                            break;
                        } else {
                            i += 1;
                        }
                    }
                } else {
                    i += 1;
                    loop {
                        match chars.get(i) {
                            None | Some('\n') | Some('\r') => {
                                return Err("unterminated string".into())
                            }
                            Some('\\') => i += 2,
                            Some('"') => {
                                i += 1;
                                break;
                            }
                            Some(_) => i += 1,
                        }
                    }
                }
            }
            '.' => {
                if chars[i..].starts_with(&['.', '.', '.']) {
                    tokens.push(Token::Spread);
                    i += 3;
                } else {
                    i += 1; // part of a number
                }
            }
            c if c.is_ascii_digit() || c == '-' || c == '+' => i += 1,
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                tokens.push(Token::Name(chars[start..i].iter().collect()));
            }
            '{' | '}' | '(' | ')' | '[' | ']' | ':' | '@' | '$' | '!' | '=' | '|' | '&' => {
                tokens.push(Token::Punct(c));
                i += 1;
            }
            other => {
                return Err(format!(
                    "unexpected character `{other}` in the GraphQL document"
                ))
            }
        }
    }
    Ok(tokens)
}

/// Check a `linear_graphql` document: one operation, no subscriptions, and mutations limited to
/// [`LINEAR_ALLOWED_MUTATIONS`] (every root field, aliases resolved, must be on the list).
pub fn check_linear_document(query: &str) -> Result<(), String> {
    let tokens = tokenize(query)?;
    // Operations are the top-level `query | mutation | subscription` keywords or a bare `{`.
    // Inside `( … )` (variable definitions and arguments) keywords are names and `{` opens an
    // object value, so neither starts an operation.
    let mut depth = 0i32;
    let mut parens = 0i32;
    let mut operations: Vec<(usize, &str)> = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::Punct('(') => parens += 1,
            Token::Punct(')') => {
                parens -= 1;
                if parens < 0 {
                    return Err("the GraphQL document has unbalanced parentheses".into());
                }
            }
            Token::Punct('{') => {
                if depth == 0
                    && parens == 0
                    && !matches!(
                        tokens.get(index.wrapping_sub(1)),
                        Some(Token::Punct(')')) | Some(Token::Name(_))
                    )
                {
                    operations.push((index, "query"));
                }
                depth += 1;
            }
            Token::Punct('}') => {
                depth -= 1;
                if depth < 0 {
                    return Err("the GraphQL document has unbalanced braces".into());
                }
            }
            Token::Name(name) if depth == 0 && parens == 0 => match name.as_str() {
                "query" => operations.push((index, "query")),
                "mutation" => operations.push((index, "mutation")),
                "subscription" => operations.push((index, "subscription")),
                _ => {}
            },
            _ => {}
        }
    }
    if depth != 0 || parens != 0 {
        return Err("the GraphQL document has unbalanced braces".into());
    }
    match operations.as_slice() {
        [] => Err("linear_graphql needs one query or mutation".into()),
        [(_, "subscription")] => Err("linear_graphql does not allow subscriptions".into()),
        [(_, "query")] => Ok(()),
        [(start, "mutation")] => check_linear_mutation(&tokens[*start..]),
        _ => Err("linear_graphql accepts a single operation per call".into()),
    }
}

fn check_linear_mutation(tokens: &[Token]) -> Result<(), String> {
    // Skip the operation header up to the selection set that opens the operation.
    let mut i = 0;
    let mut parens = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Punct('(') => parens += 1,
            Token::Punct(')') => parens -= 1,
            Token::Punct('{') if parens == 0 => break,
            _ => {}
        }
        i += 1;
    }
    let (mut depth, mut parens) = (0i32, 0i32);
    let mut fields: Vec<&str> = Vec::new();
    let mut previous: Option<&Token> = None;
    while i < tokens.len() {
        let token = &tokens[i];
        match token {
            Token::Punct('{') => depth += 1,
            Token::Punct('}') => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Token::Punct('(') => parens += 1,
            Token::Punct(')') => parens -= 1,
            Token::Spread if depth >= 1 => {
                return Err("fragments are not allowed in a mutation".into());
            }
            Token::Name(name) if depth == 1 && parens == 0 => {
                let after_directive_marker = matches!(previous, Some(Token::Punct('@')));
                let is_alias = matches!(tokens.get(i + 1), Some(Token::Punct(':')));
                if !after_directive_marker && !is_alias {
                    fields.push(name);
                }
            }
            _ => {}
        }
        previous = Some(token);
        i += 1;
    }
    if fields.is_empty() {
        return Err("the mutation has no fields".into());
    }
    for field in fields {
        if !LINEAR_ALLOWED_MUTATIONS.contains(&field) {
            return Err(format!(
                "linear_graphql does not allow the `{field}` mutation; allowed: {}",
                LINEAR_ALLOWED_MUTATIONS.join(", ")
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = "Acme/Widgets";

    fn allowed(method: &str, path: &str) -> bool {
        check_github_request(REPO, method, path).is_ok()
    }

    #[test]
    fn github_tool_keeps_an_implementation_run_working() {
        for (m, p) in [
            ("GET", "/repos/acme/widgets/issues/42"),
            ("GET", "/repos/Acme/Widgets/pulls?state=open&per_page=50"),
            ("GET", "/repos/acme/widgets/commits/abc123/check-runs"),
            ("GET", "/repos/acme/widgets/actions/runs/9/jobs"),
            ("GET", "/repos/acme/widgets/contents/src/lib.rs?ref=main"),
            ("POST", "/repos/acme/widgets/issues/42/comments"),
            ("POST", "/repos/acme/widgets/issues/42/labels"),
            ("POST", "/repos/acme/widgets/issues"),
            ("PATCH", "/repos/acme/widgets/issues/42"),
            ("PATCH", "/repos/acme/widgets/pulls/7"),
            ("PATCH", "/repos/acme/widgets/issues/comments/1234"),
            ("post", "/repos/acme/widgets/issues/42/comments"),
        ] {
            assert!(allowed(m, p), "{m} {p} should be allowed");
        }
    }

    #[test]
    fn github_tool_cannot_merge_delete_administer_or_leave_the_repository() {
        for (m, p) in [
            // Merging stays a human decision.
            ("PUT", "/repos/acme/widgets/pulls/7/merge"),
            ("POST", "/repos/acme/widgets/merges"),
            // Publishing a branch is the human-confirmed "Create pull request" action.
            ("POST", "/repos/acme/widgets/pulls"),
            ("PUT", "/repos/acme/widgets/pulls/7/update-branch"),
            ("POST", "/repos/acme/widgets/pulls/7/reviews"),
            // Destruction and administration.
            ("DELETE", "/repos/acme/widgets"),
            ("DELETE", "/repos/acme/widgets/git/refs/heads/main"),
            ("PATCH", "/repos/acme/widgets"),
            ("POST", "/repos/acme/widgets/hooks"),
            ("GET", "/repos/acme/widgets/actions/secrets"),
            ("GET", "/repos/acme/widgets/collaborators"),
            ("PUT", "/repos/acme/widgets/collaborators/mallory"),
            ("POST", "/repos/acme/widgets/transfer"),
            ("POST", "/repos/acme/widgets/dispatches"),
            ("PUT", "/repos/acme/widgets/branches/main/protection"),
            // Other repositories and account-level endpoints reachable with the same token.
            ("GET", "/repos/acme/other-private/contents/secrets.env"),
            ("GET", "/repos/acme/widgets-evil/issues"),
            ("GET", "/user"),
            ("GET", "/user/repos"),
            ("DELETE", "/repos/victim/repo"),
            ("GET", "/orgs/acme/members"),
            ("POST", "/user/repos"),
            // Smuggling.
            ("GET", "/repos/acme/widgets/../other/issues"),
            ("GET", "/repos/acme/widgets/%2e%2e/other/issues"),
            ("GET", "/repos/acme/widgets/issues/42/../../../../user"),
            ("GET", "//evil.example/repos/acme/widgets/issues"),
            ("GET", "https://evil.example/repos/acme/widgets/issues"),
            ("GET", "/repos/acme/widgets/issues\n/42"),
            ("GET", "repos/acme/widgets/issues"),
        ] {
            assert!(!allowed(m, p), "{m} {p} must be refused");
        }
    }

    #[test]
    fn github_tool_needs_a_configured_repository() {
        assert!(check_github_request("", "GET", "/repos/a/b/issues").is_err());
        assert!(check_github_request("noslash", "GET", "/repos/a/b/issues").is_err());
    }

    fn linear(query: &str) -> Result<(), String> {
        check_linear_document(query)
    }

    #[test]
    fn linear_queries_are_read_only_and_allowed() {
        assert!(linear("query Q($id: String!) { issue(id: $id) { id title } }").is_ok());
        assert!(linear("{ viewer { id } }").is_ok());
        assert!(linear("query { issues(filter: { title: { contains: \"mutation { issueDelete }\" } }) { nodes { id } } }").is_ok());
        assert!(linear("# mutation { issueDelete(id: \"1\") }\nquery { viewer { id } }").is_ok());
        assert!(linear("query { a: viewer { id } }\nfragment F on Issue { id }").is_ok());
        // Keywords as variable names and object defaults are not further operations.
        assert!(
            linear("query Q($query: String!) { issueSearch(query: $query) { nodes { id } } }")
                .is_ok()
        );
        assert!(linear(
            "query Q($f: IssueFilter = { title: {} }) { issues(filter: $f) { nodes { id } } }"
        )
        .is_ok());
        // Closing more than was opened cannot be used to hide a second operation.
        assert!(linear("query { a } } mutation { issueDelete(id: \"1\") { success } } {").is_err());
    }

    #[test]
    fn linear_mutations_are_limited_to_the_allowlist() {
        assert!(linear("mutation($b: String!, $i: String!) { commentCreate(input: { issueId: $i, body: $b }) { success } }").is_ok());
        assert!(linear(
            "mutation { issueUpdate(id: \"x\", input: { stateId: \"s\" }) { success } }"
        )
        .is_ok());
        assert!(linear("mutation { commentCreate(input: {}) { success } issueUpdate(id: \"x\", input: {}) { success } }").is_ok());
        for bad in [
            "mutation { issueDelete(id: \"x\") { success } }",
            "mutation { a: issueDelete(id: \"x\") { success } }",
            "mutation { commentCreate(input: {}) { success } projectDelete(id: \"p\") { success } }",
            "mutation Named { teamDelete(id: \"t\") { success } }",
            "mutation { ...F }\nfragment F on Mutation { issueDelete(id: \"x\") { success } }",
            "mutation { issueArchive(id: \"x\") { success } }",
            "mutation { apiKeyCreate(input: {}) { success } }",
            "subscription { issueUpdated { id } }",
            "query { viewer { id } }\nmutation { issueDelete(id: \"x\") { success } }",
            "mutation { issueDelete(id: \"x\") { success }",
            "mutation { commentCreate(input: { body: \"unterminated }) { success } }",
            "",
            "viewer",
        ] {
            assert!(linear(bad).is_err(), "must be refused: {bad}");
        }
    }
}
