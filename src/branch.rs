use std::str::FromStr;

use nom::{
    AsChar, IResult, Parser,
    branch::alt,
    bytes::complete::{tag, take_till1, take_until1, take_while1},
    character::complete::{self, hex_digit1, multispace0, not_line_ending},
    combinator::{map, opt},
    error::{ParseError, context},
    sequence::{delimited, preceded},
};

use anyhow::anyhow;

fn parse_current(value: &str) -> IResult<&str, bool> {
    context("current", map(opt(complete::char('*')), |c| c.is_some())).parse(value)
}

fn parse_name(value: &str) -> IResult<&str, String> {
    context(
        "name",
        map(
            alt((
                delimited(
                    tag("("),
                    take_while1(|c: char| c.is_ascii_alphanumeric() || c.is_ascii_whitespace()),
                    tag(")"),
                ),
                take_till1(AsChar::is_space),
            )),
            String::from,
        ),
    )
    .parse(value)
}

fn parse_commit_sha(value: &str) -> IResult<&str, String> {
    context("commit_sha", map(hex_digit1, String::from)).parse(value)
}

fn parse_commit_message(value: &str) -> IResult<&str, String> {
    context("commit_message", map(not_line_ending, String::from)).parse(value)
}

fn parse_branch_pointer(value: &str) -> IResult<&str, String> {
    context(
        "ref",
        map(preceded(tag("-> "), not_line_ending), String::from),
    )
    .parse(value)
}

fn parse_upstream_branch(value: &str) -> IResult<&str, Option<String>> {
    context(
        "upstream_branch",
        opt(delimited(
            tag("["),
            map(take_until1("]"), String::from),
            tag("]"),
        )),
    )
    .parse(value)
}

pub trait Branch {
    fn name(&self) -> &str;
}

#[derive(Default, Clone)]
pub struct LocalBranch {
    pub name: String,
    pub current: bool,
    pub commit_sha: String,
    pub upstream_branch: Option<String>,
    pub commit_message: String,
}

impl Branch for LocalBranch {
    fn name(&self) -> &str {
        &self.name
    }
}

impl FromStr for LocalBranch {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (current, name, commit_sha, upstream_branch, commit_message) = (
            ws(parse_current),
            ws(parse_name),
            ws(parse_commit_sha),
            ws(parse_upstream_branch),
            parse_commit_message,
        )
            .parse(s)
            .map_err(|e| anyhow!("Failed to parse branch line: {}", e.to_owned()))?
            .1;

        Ok(Self {
            name,
            current,
            commit_sha,
            upstream_branch,
            commit_message,
        })
    }
}

#[derive(Clone)]
pub enum RemoteBranchRef {
    Branch(String),
    Commit { sha: String, message: String },
}

#[derive(Clone)]
pub struct RemoteBranch {
    pub name: String,
    pub reference: RemoteBranchRef,
}

impl RemoteBranch {
    fn parse_reference(input: &str) -> IResult<&str, RemoteBranchRef> {
        alt((
            map(
                (ws(parse_commit_sha), parse_commit_message),
                |(sha, message)| RemoteBranchRef::Commit { sha, message },
            ),
            map(ws(parse_branch_pointer), |branch_ref| {
                RemoteBranchRef::Branch(branch_ref)
            }),
        ))
        .parse(input)
    }
}

impl Branch for RemoteBranch {
    fn name(&self) -> &str {
        &self.name
    }
}

impl FromStr for RemoteBranch {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (name, reference) = (ws(parse_name), Self::parse_reference)
            .parse(s)
            .map_err(|e| anyhow!("Failed to parse remote branch line: {}", e.to_owned()))?
            .1;

        Ok(Self { name, reference })
        // let (name, commit_sha, commit_message) =
        //     (ws(parse_name), ws(parse_commit_sha), parse_commit_message)
        //         .parse(s)
        //         .map_err(|e| anyhow!("Failed to parse remote branch line: {}", e.to_owned()))?
        //         .1;

        // Ok(Self {
        //     name,
        //     commit_sha,
        //     commit_message,
        // })
    }
}

pub fn ws<'a, O, E: ParseError<&'a str>, F>(inner: F) -> impl Parser<&'a str, Output = O, Error = E>
where
    F: Parser<&'a str, Output = O, Error = E>,
{
    delimited(multispace0, inner, multispace0)
}
