use chrono::NaiveTime;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt::Display;

/// Import from Wordpress API
pub struct WordpressAPI {
    base_url: String,
    client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WordpressAPIError {
    SerdeParsingError,
    ReqwestError,
    InvalidURL,
    NotFound,
    UnexpectedResponse,
    Unsupported(String),
}

#[derive(Debug)]
pub enum PostDataType {
    PostPreviews(Vec<PostPreview>),
    FullPosts(Vec<Post>),
}

#[derive(Debug)]
pub struct PostData {
    pub number_of_records: usize,
    pub data: PostDataType,
}

impl PostData {
    /// Returns Err if postdatatypes doesn't match
    pub fn push_posts(&mut self, posts: PostDataType) -> Result<(), ()> {
        match (&mut self.data, posts) {
            (PostDataType::PostPreviews(data), PostDataType::PostPreviews(mut new_data)) => {
                data.append(&mut new_data);
                Ok(())
            }
            (PostDataType::FullPosts(data), PostDataType::FullPosts(mut new_data)) => {
                data.append(&mut new_data);
                Ok(())
            }
            _ => Err(()),
        }
    }

    /// Removes all elements exceeding the limit
    pub fn cap_to(&mut self, limit: usize) {
        match &mut self.data {
            PostDataType::PostPreviews(data) => data.truncate(limit),
            PostDataType::FullPosts(data) => data.truncate(limit),
        }
    }

    /// Number of collected posts/previews.
    pub fn len(&self) -> usize {
        match &self.data {
            PostDataType::PostPreviews(data) => data.len(),
            PostDataType::FullPosts(data) => data.len(),
        }
    }

    /// Whether no posts/previews have been collected yet.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WordpressUser {
    pub id: usize,
    pub name: String,
    pub url: String,
    pub description: String,
    pub link: String,
    pub slug: String,
    /// Custom fields of the user (Advanced Custom Fields). Verfassungsblog keeps the translated
    /// biographies here, while `description` often only holds the (German) default one.
    #[serde(default, deserialize_with = "deserialize_acf")]
    pub acf: Option<WordpressUserAcf>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct WordpressUserAcf {
    #[serde(default, deserialize_with = "deserialize_lenient_string")]
    pub author_eng_desc: Option<String>,
    #[serde(default, deserialize_with = "deserialize_lenient_string")]
    pub author_fr_desc: Option<String>,
    #[serde(default, deserialize_with = "deserialize_lenient_string")]
    pub orcid: Option<String>,
    #[serde(default, deserialize_with = "deserialize_lenient_string")]
    pub gndid: Option<String>,
    #[serde(default, deserialize_with = "deserialize_lenient_string")]
    pub rorid: Option<String>,
}

/// Persistent identifiers of a WordPress user (plain ids without URL prefix).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct WpAuthorIds {
    pub orcid: Option<String>,
    pub gnd: Option<String>,
    pub ror: Option<String>,
}

/// A biography (still as HTML) fetched from WordPress. `lang` is only set if WordPress tells us
/// the language (e.g. via the name of the custom field), otherwise it has to be detected.
#[derive(Debug, Clone, PartialEq)]
pub struct WpBio {
    pub lang: Option<language::Language>,
    pub html: String,
}

impl WordpressUser {
    /// ORCID, GND and ROR from the custom fields; empty values are dropped.
    pub fn identifiers(&self) -> WpAuthorIds {
        // Editors sometimes paste the full URL (https://orcid.org/0000-...); keep only the id.
        let clean = |v: &Option<String>| {
            v.as_deref()
                .map(|v| v.trim().trim_end_matches('/'))
                .and_then(|v| v.rsplit('/').next())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        match &self.acf {
            Some(acf) => WpAuthorIds {
                orcid: clean(&acf.orcid),
                gnd: clean(&acf.gndid),
                ror: clean(&acf.rorid),
            },
            None => WpAuthorIds::default(),
        }
    }

    /// All non-empty biographies of this user: the default `description` and the translated ones
    /// from the custom fields.
    pub fn biographies(&self) -> Vec<WpBio> {
        let mut bios = vec![WpBio {
            lang: None,
            html: self.description.clone(),
        }];
        if let Some(acf) = &self.acf {
            bios.push(WpBio {
                lang: Some(language::Language::EnUs),
                html: acf.author_eng_desc.clone().unwrap_or_default(),
            });
            bios.push(WpBio {
                lang: Some(language::Language::FrFr),
                html: acf.author_fr_desc.clone().unwrap_or_default(),
            });
        }
        bios.retain(|b| !b.html.trim().is_empty());
        bios
    }
}

#[derive(Debug, Default)]
pub enum WordpressAPIContext {
    #[default]
    View,
    Embed,
}

impl Display for WordpressAPIContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let str = match self {
            WordpressAPIContext::View => String::from("view"),
            WordpressAPIContext::Embed => String::from("embed"),
        };
        write!(f, "{}", str)
    }
}

/// Longest request URL we send; longer URLs are rejected by many servers.
const MAX_URL_LENGTH: usize = 2000;
/// URL length reserved for included categories before excluded ones are dropped from the query.
const MIN_INCLUDE_CHUNK_LENGTH: usize = 100;
/// URL length of the `,` between ids once reqwest has url-encoded it (`%2C`).
const ENCODED_SEPARATOR_LENGTH: usize = 3;
/// URL length reserved for `&page=N`, which is only added per request.
const PAGE_PARAM_LENGTH: usize = "&page=".len() + 5;
/// Fields requested for previews when posts have to be filtered by category after fetching:
/// the `embed` context doesn't return `categories`, so `view` is limited to these fields instead.
const PREVIEW_FIELDS: &str = "id,date,link,slug,type,title,author,excerpt,categories";
/// Fields requested to count the posts matching a query.
const COUNT_FIELDS: &str = "id,categories";

/// Length of `value` once url-encoded by reqwest's `.query()`.
fn encoded_length(value: &str) -> usize {
    url::form_urlencoded::byte_serialize(value.as_bytes())
        .map(str::len)
        .sum()
}

/// URL length of the query parameters `&{key}={value}`, url-encoded.
fn query_length(query: &[(String, String)]) -> usize {
    query
        .iter()
        .map(|(key, value)| encoded_length(key) + encoded_length(value) + 2)
        .sum()
}

fn join_ids(ids: &[usize]) -> String {
    ids.iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// URL length of `&{key}={id},{id},...`, url-encoded.
fn id_param_length(key: &str, ids: &[usize]) -> usize {
    key.len()
        + 2
        + ids.iter().map(|id| id.to_string().len()).sum::<usize>()
        + ENCODED_SEPARATOR_LENGTH * ids.len().saturating_sub(1)
}

/// How the category filters of a posts request are fit into the URL length limit.
#[derive(Debug, PartialEq)]
struct CategoryQueryPlan {
    /// Comma-separated included category ids; one request series is made per chunk. Empty if
    /// no categories are included. Never contains an empty chunk, since `categories=` would
    /// disable the filter.
    include_chunks: Vec<String>,
    /// Whether the excluded categories fit into the query. Otherwise posts have to be filtered
    /// after fetching.
    exclude_in_query: bool,
}

impl CategoryQueryPlan {
    /// `url_length` is the url-encoded length of the URL including all other query parameters.
    fn new(url_length: usize, include: &[usize], exclude: &[usize]) -> Self {
        let remaining = MAX_URL_LENGTH.saturating_sub(url_length);
        let include_length = if include.is_empty() {
            0
        } else {
            id_param_length("categories", include)
        };
        let exclude_length = if exclude.is_empty() {
            0
        } else {
            id_param_length("categories_exclude", exclude)
        };

        // Prefer sending the excluded categories along, as long as some room for included
        // categories is left.
        let exclude_in_query = include_length + exclude_length <= remaining
            || exclude_length + MIN_INCLUDE_CHUNK_LENGTH <= remaining;
        let available = if exclude_in_query {
            remaining.saturating_sub(exclude_length)
        } else {
            remaining
        };

        Self {
            include_chunks: chunk_ids(include, available.saturating_sub("&categories=".len())),
            exclude_in_query,
        }
    }
}

/// Joins `ids` into comma-separated lists whose url-encoded length is at most `max_length`. Each
/// list holds at least one id, even if that alone exceeds `max_length`.
fn chunk_ids(ids: &[usize], max_length: usize) -> Vec<String> {
    let mut chunks = vec![];
    let mut chunk = String::new();
    let mut chunk_length = 0;
    for id in ids {
        let id = id.to_string();
        if !chunk.is_empty() && chunk_length + ENCODED_SEPARATOR_LENGTH + id.len() > max_length {
            chunks.push(std::mem::take(&mut chunk));
            chunk_length = 0;
        }
        if !chunk.is_empty() {
            chunk.push(',');
            chunk_length += ENCODED_SEPARATOR_LENGTH;
        }
        chunk.push_str(&id);
        chunk_length += id.len();
    }
    if !chunk.is_empty() {
        chunks.push(chunk);
    }
    chunks
}

/// A post as returned by `/wp/v2/posts`, in whichever context or with whichever fields.
trait ListedPost {
    fn id(&self) -> usize;
    fn categories(&self) -> &[usize];
}

impl ListedPost for Post {
    fn id(&self) -> usize {
        self.id
    }
    fn categories(&self) -> &[usize] {
        &self.categories
    }
}

impl ListedPost for PostPreview {
    fn id(&self) -> usize {
        self.id
    }
    fn categories(&self) -> &[usize] {
        &self.categories
    }
}

/// Only the fields needed to count posts, see [`COUNT_FIELDS`].
#[derive(Deserialize)]
struct PostCategories {
    id: usize,
    categories: Vec<usize>,
}

impl ListedPost for PostCategories {
    fn id(&self) -> usize {
        self.id
    }
    fn categories(&self) -> &[usize] {
        &self.categories
    }
}

/// Drops posts that are in one of the `excluded` categories or were already `seen` (a post in
/// several included categories is returned once per chunk it matches). Records the kept ids in
/// `seen`.
fn keep_new_posts<T: ListedPost>(
    posts: &mut Vec<T>,
    seen: &mut HashSet<usize>,
    excluded: Option<&HashSet<usize>>,
) {
    posts.retain(|post| {
        let is_excluded = excluded.is_some_and(|excluded| {
            post.categories()
                .iter()
                .any(|category| excluded.contains(category))
        });
        !is_excluded && seen.insert(post.id())
    });
}

/// Posts collected by [`WordpressAPI::collect_posts`].
struct CollectedPosts<T> {
    posts: Vec<T>,
    /// Sum of `X-WP-Total` over all chunks, i.e. before deduplication and manual filtering.
    header_total: usize,
    /// Whether all pages of all chunks were fetched, i.e. the limit wasn't hit.
    complete: bool,
}

/// Reads a numeric response header such as `X-WP-Total`.
fn header_usize(response: &reqwest::Response, name: &str) -> Result<usize, WordpressAPIError> {
    let Some(value) = response.headers().get(name) else {
        error!("{} is missing in posts response", name);
        debug!("{:?}", response);
        return Err(WordpressAPIError::UnexpectedResponse);
    };
    value
        .to_str()
        .ok()
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| {
            error!("Error parsing {} as usize.", name);
            debug!("{:?}", response);
            WordpressAPIError::UnexpectedResponse
        })
}

impl WordpressAPI {
    pub fn new(base_url: String) -> Result<Self, WordpressAPIError> {
        Ok(WordpressAPI {
            base_url,
            client: WordpressAPI::build_client(10000)?,
        })
    }

    fn build_client(timeout: u64) -> Result<reqwest::Client, WordpressAPIError> {
        match reqwest::ClientBuilder::new()
            .timeout(std::time::Duration::from_millis(timeout))
            .build()
        {
            Ok(client) => Ok(client),
            Err(e) => {
                eprintln!("Error building client: {}", e);
                Err(WordpressAPIError::ReqwestError)
            }
        }
    }

    pub async fn get_user(&self, id: usize) -> Result<WordpressUser, WordpressAPIError> {
        let url = format!("https://{}/wp-json/wp/v2/users/{}", self.base_url, id);

        let client = self.client.clone();
        let request = client.request(reqwest::Method::GET, &url);

        match request.send().await {
            Ok(response) => {
                debug!("Got user response status: {:?}", response.status());
                if response.status() == 404 {
                    Err(WordpressAPIError::NotFound)
                } else {
                    match response.json::<WordpressUser>().await {
                        Ok(res) => Ok(res),
                        Err(e) => {
                            error!("Couldn't parse user response from wordpress: {}", e);
                            Err(WordpressAPIError::SerdeParsingError)
                        }
                    }
                }
            }
            Err(e) => {
                error!("Couldn't parse user response from wordpress: {}", e);
                Err(WordpressAPIError::ReqwestError)
            }
        }
    }

    /// Adds the data the post endpoints don't embed (co-authors and resolved tags). Failures are
    /// ignored and just leave the respective fields empty.
    pub async fn enrich_post(&self, post: &mut Post) {
        let _ = self.add_coauthors(post).await;
        let _ = self.add_resolved_tags(post).await;
    }

    pub async fn add_coauthors(&self, post: &mut Post) -> Result<(), WordpressAPIError> {
        let id = &post.id;
        let url = format!(
            "https://{}/wp-json/coauthors/v1/coauthors/?post_id={}",
            self.base_url, id
        );

        let client = self.client.clone();
        let request = client.request(reqwest::Method::GET, &url);

        debug!("coauthor request send for post {}", post.id);

        match request.send().await {
            Ok(response) => {
                debug!("Got coauthor response status: {:?}", response.status());
                match response.json::<Vec<CoAuthor>>().await {
                    Ok(res) => {
                        debug!("Got coauthors for post {}", post.id);
                        post.coauthors = Some(res);
                        Ok(())
                    }
                    Err(e) => {
                        error!(
                            "Couldn't parse coauthor response from wordpress: {}, for post {}",
                            e, post.id
                        );
                        Err(WordpressAPIError::SerdeParsingError)
                    }
                }
            }

            Err(e) => {
                error!(
                    "Couldn't parse coauthor response from wordpress: {}, for post {}",
                    e, post.id
                );
                Err(WordpressAPIError::ReqwestError)
            }
        }
    }

    /// Resolves the tags of `post` into tag names and stores them in `post.tags_resolved`.
    ///
    /// Asks WordPress for the tags assigned to the post (`/tags?post=<id>`, all pages). Does
    /// nothing if the post has no tags. Names are ordered like `post.tags`; tags missing from
    /// that list sort last.
    pub async fn add_resolved_tags(&self, post: &mut Post) -> Result<(), WordpressAPIError> {
        if post.tags.is_empty() {
            return Ok(());
        }
        let url = format!("https://{}/wp-json/wp/v2/tags", self.base_url);

        let mut tags: Vec<Tag> = Vec::new();
        let mut page = 1;
        loop {
            let query = [
                ("post", post.id.to_string()),
                ("per_page", "100".to_string()),
                ("page", page.to_string()),
            ];
            let response = match self.client.get(&url).query(&query).send().await {
                Ok(response) => response,
                Err(e) => {
                    error!("Couldn't fetch tags for post {}: {}", post.id, e);
                    return Err(WordpressAPIError::ReqwestError);
                }
            };
            if !response.status().is_success() {
                error!(
                    "Couldn't fetch tags for post {}: status {}",
                    post.id,
                    response.status()
                );
                return Err(WordpressAPIError::UnexpectedResponse);
            }
            let total_pages = response
                .headers()
                .get("X-WP-TotalPages")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(1);
            match response.json::<Vec<Tag>>().await {
                Ok(mut page_tags) => tags.append(&mut page_tags),
                Err(e) => {
                    error!("Couldn't parse tags for post {}: {}", post.id, e);
                    return Err(WordpressAPIError::SerdeParsingError);
                }
            }
            if page >= total_pages {
                break;
            }
            page += 1;
        }

        tags.sort_by_key(|tag| {
            post.tags
                .iter()
                .position(|id| *id == tag.id)
                .unwrap_or(usize::MAX)
        });
        post.tags_resolved = Some(tags.into_iter().map(|tag| tag.name).collect());
        Ok(())
    }

    pub async fn get_posts(
        &self,
        context: WordpressAPIContext,
        search: Option<String>,
        after: Option<chrono::NaiveDate>,
        modified_after: Option<chrono::NaiveDate>,
        before: Option<chrono::NaiveDate>,
        modified_before: Option<chrono::NaiveDate>,
        slug: Option<String>,
        categories: Option<Vec<usize>>,
        categories_exclude: Option<Vec<usize>>,
        limit: Option<usize>,
    ) -> Result<PostData, WordpressAPIError> {
        let url = format!("https://{}/wp-json/wp/v2/posts", self.base_url);
        let categories = categories.unwrap_or_default();
        let categories_exclude = categories_exclude.unwrap_or_default();

        // Filters shared by the post requests and the count requests.
        let mut filters: Vec<(String, String)> = Vec::new();
        if let Some(search) = search {
            filters.push(("search".to_string(), search));
        }
        // Warning: The Wordpress Documentation is wrong: the API expects a DateTime, not a Date!
        for (name, date) in [
            ("after", after),
            ("modified_after", modified_after),
            ("before", before),
            ("modified_before", modified_before),
        ] {
            if let Some(date) = date {
                filters.push((
                    name.to_string(),
                    date.and_time(NaiveTime::default()).to_string(),
                ));
            }
        }
        if let Some(slug) = slug {
            filters.push(("slug".to_string(), slug));
        }

        // URL length without the category filters. Reserves room for the longest of the
        // `context`/`_fields` variants used below, so the plan holds for every request.
        let url_length = url.len()
            + query_length(&filters)
            + query_length(&[
                ("context".to_string(), "embed".to_string()),
                ("_fields".to_string(), PREVIEW_FIELDS.to_string()),
                ("per_page".to_string(), "100".to_string()),
            ])
            + PAGE_PARAM_LENGTH;
        let plan = CategoryQueryPlan::new(url_length, &categories, &categories_exclude);

        if plan.exclude_in_query && !categories_exclude.is_empty() {
            filters.push((
                "categories_exclude".to_string(),
                join_ids(&categories_exclude),
            ));
        }
        let excluded: Option<HashSet<usize>> = (!plan.exclude_in_query
            && !categories_exclude.is_empty())
        .then(|| categories_exclude.iter().copied().collect());
        // The header totals only match the collected posts if a single request series was made
        // and nothing was filtered out afterwards.
        let header_total_is_exact = plan.include_chunks.len() <= 1 && excluded.is_none();

        // One request series per chunk of included categories, or a single one without a
        // category filter.
        let category_filters: Vec<Option<String>> = if plan.include_chunks.is_empty() {
            vec![None]
        } else {
            plan.include_chunks.into_iter().map(Some).collect()
        };

        let query_with = |context: &str, fields: Option<&str>| {
            let mut query = vec![
                ("context".to_string(), context.to_string()),
                ("per_page".to_string(), "100".to_string()),
            ];
            if let Some(fields) = fields {
                query.push(("_fields".to_string(), fields.to_string()));
            }
            query.extend(filters.iter().cloned());
            query
        };

        let (data, header_total, complete) = match context {
            WordpressAPIContext::View => {
                let collected = self
                    .collect_posts::<Post>(
                        &url,
                        &query_with("view", None),
                        &category_filters,
                        excluded.as_ref(),
                        limit,
                    )
                    .await?;
                (
                    PostDataType::FullPosts(collected.posts),
                    collected.header_total,
                    collected.complete,
                )
            }
            WordpressAPIContext::Embed => {
                // The embed context doesn't include the categories needed to filter manually.
                let query = if excluded.is_some() {
                    query_with("view", Some(PREVIEW_FIELDS))
                } else {
                    query_with("embed", None)
                };
                let collected = self
                    .collect_posts::<PostPreview>(
                        &url,
                        &query,
                        &category_filters,
                        excluded.as_ref(),
                        limit,
                    )
                    .await?;
                (
                    PostDataType::PostPreviews(collected.posts),
                    collected.header_total,
                    collected.complete,
                )
            }
        };
        let mut all_posts = PostData {
            number_of_records: 0,
            data,
        };

        all_posts.number_of_records = if header_total_is_exact {
            header_total
        } else if complete {
            all_posts.len()
        } else {
            // The limit was hit before all posts were seen, so count the remaining ones with a
            // lightweight request series.
            self.collect_posts::<PostCategories>(
                &url,
                &query_with("view", Some(COUNT_FIELDS)),
                &category_filters,
                excluded.as_ref(),
                None,
            )
            .await?
            .posts
            .len()
        };

        if let Some(limit) = limit {
            all_posts.cap_to(limit);
        }

        Ok(all_posts)
    }

    /// Fetches all pages of `query` once per entry of `category_filters` (a comma-separated list
    /// for the `categories` parameter, or `None` for no category filter). Drops duplicates and
    /// posts in `excluded` categories, and stops once `limit` posts are collected.
    async fn collect_posts<T: ListedPost + serde::de::DeserializeOwned>(
        &self,
        url: &str,
        query: &[(String, String)],
        category_filters: &[Option<String>],
        excluded: Option<&HashSet<usize>>,
        limit: Option<usize>,
    ) -> Result<CollectedPosts<T>, WordpressAPIError> {
        let mut collected = CollectedPosts {
            posts: Vec::new(),
            header_total: 0,
            complete: true,
        };
        let mut seen = HashSet::new();

        'chunks: for (chunk_index, category_filter) in category_filters.iter().enumerate() {
            let mut chunk_query = query.to_vec();
            if let Some(chunk) = category_filter {
                chunk_query.push(("categories".to_string(), chunk.clone()));
            }

            let mut page = 1;
            loop {
                let mut page_query = chunk_query.clone();
                page_query.push(("page".to_string(), page.to_string()));

                debug!("Current query is: {:?}", page_query);

                let response = match self.client.get(url).query(&page_query).send().await {
                    Ok(response) => response,
                    Err(e) => {
                        error!("Error fetching posts: {}", e);
                        return Err(WordpressAPIError::ReqwestError);
                    }
                };
                if response.status() == 400 {
                    // We will get a bad requests if no posts are found at this page
                    return Err(WordpressAPIError::NotFound);
                }
                let num_of_records = header_usize(&response, "X-WP-Total")?;
                let num_of_pages = header_usize(&response, "X-WP-TotalPages")?;

                let mut posts = match response.json::<Vec<T>>().await {
                    Ok(posts) => posts,
                    Err(e) => {
                        error!("Error parsing posts: {}", e);
                        return Err(WordpressAPIError::SerdeParsingError);
                    }
                };
                keep_new_posts(&mut posts, &mut seen, excluded);
                collected.posts.append(&mut posts);

                // `X-WP-Total` is the total matching this (chunk) query and is repeated on
                // every page, so only count it once per chunk (on the first page).
                if page == 1 {
                    collected.header_total += num_of_records;
                }

                let last_page = page >= num_of_pages;
                // Stop once we have collected enough posts to satisfy the limit; the caller
                // trims any overshoot from the last page.
                if limit.is_some_and(|limit| collected.posts.len() >= limit) {
                    collected.complete = last_page && chunk_index + 1 == category_filters.len();
                    break 'chunks;
                }
                if last_page {
                    break;
                }
                page += 1;
            }
        }

        Ok(collected)
    }

    pub async fn get_post(&self, id: usize) -> Result<Post, WordpressAPIError> {
        let url = format!("https://{}/wp-json/wp/v2/posts/{}", self.base_url, id);
        let client = self.client.clone();
        let response = client.get(&url).send().await;
        match response {
            Ok(response) => match response.json::<Post>().await {
                Ok(mut post) => {
                    self.enrich_post(&mut post).await;
                    Ok(post)
                }
                Err(e) => {
                    eprintln!("Error parsing post {}: {}", id, e);
                    Err(WordpressAPIError::SerdeParsingError)
                }
            },
            Err(e) => {
                eprintln!("Error fetching post {}: {}", id, e);
                Err(WordpressAPIError::ReqwestError)
            }
        }
    }

    pub async fn get_category_tree(&self) -> Result<CategoryTree, WordpressAPIError> {
        let mut categories = Vec::new();

        println!("Fetching all categories from Wordpress API");

        let mut page = 1;
        loop {
            println!("Fetching all categories from page {}", page);
            let mut new_categories = self
                .get_categories(
                    Some(page),
                    Some(100),
                    None,
                    None,
                    None,
                    None,
                    Some(true),
                    None,
                    None,
                )
                .await?;
            if new_categories.is_empty() {
                println!("No more categories found, stopping");
                break;
            }
            categories.append(&mut new_categories);
            page += 1;
        }

        Ok(CategoryTree::from(categories))
    }

    pub async fn get_categories(
        &self,
        page: Option<usize>,
        per_page: Option<usize>,
        search: Option<String>,
        exclude: Option<Vec<usize>>,
        include: Option<Vec<usize>>,
        slug: Option<String>,
        hide_empty: Option<bool>,
        parent: Option<usize>,
        post: Option<usize>,
    ) -> Result<Vec<Category>, WordpressAPIError> {
        let client = self.client.clone();
        let url = format!("https://{}/wp-json/wp/v2/categories", self.base_url);
        let request = client.request(reqwest::Method::GET, &url);
        let mut query: Vec<(String, String)> = Vec::new();
        if let Some(page) = page {
            query.push(("page".to_string(), page.to_string()));
        }
        if let Some(per_page) = per_page {
            query.push(("per_page".to_string(), per_page.to_string()));
        }
        if let Some(search) = search {
            query.push(("search".to_string(), search));
        }
        if let Some(exclude) = exclude {
            query.push((
                "exclude".to_string(),
                exclude
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<String>>()
                    .join(","),
            ));
        }
        if let Some(include) = include {
            query.push((
                "include".to_string(),
                include
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<String>>()
                    .join(","),
            ));
        }
        if let Some(slug) = slug {
            query.push(("slug".to_string(), slug));
        }
        if let Some(hide_empty) = hide_empty {
            query.push(("hide_empty".to_string(), hide_empty.to_string()));
        }
        if let Some(parent) = parent {
            query.push(("parent".to_string(), parent.to_string()));
        }
        if let Some(post) = post {
            query.push(("post".to_string(), post.to_string()));
        }
        let request = request.query(&query);
        let request_started = std::time::Instant::now();
        match request.send().await {
            Ok(response) => match response.json::<Vec<Category>>().await {
                Ok(categories) => {
                    println!(
                        "Fetched {} categories in {:?}",
                        categories.len(),
                        request_started.elapsed()
                    );
                    Ok(categories)
                }
                Err(e) => {
                    eprintln!("Error parsing categories: {}", e);
                    Err(WordpressAPIError::SerdeParsingError)
                }
            },
            Err(e) => {
                eprintln!("Error fetching categories: {}", e);
                Err(WordpressAPIError::ReqwestError)
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderedContent {
    pub rendered: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostAcf {
    #[serde(default, deserialize_with = "deserialize_lenient_string")]
    pub subheadline: Option<String>,
    #[serde(default, deserialize_with = "deserialize_lenient_string")]
    pub copyright: Option<String>,
    #[serde(default, deserialize_with = "deserialize_lenient_string")]
    pub doi: Option<String>,
    #[serde(default, deserialize_with = "deserialize_lenient_string")]
    pub crossref_doi: Option<String>,
    /// All other ACF fields returned by the API
    #[serde(flatten)]
    pub other: std::collections::HashMap<String, serde_json::Value>,
}

/// ACF returns `false` (or `[]`, `null`) for empty fields. Only strings and numbers are kept;
/// anything else is treated as "not set" instead of failing the whole post.
fn deserialize_lenient_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::String(s) => Some(s),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

/// Ids are usually numbers; anything that isn't a non-negative number (or numeric string) is
/// treated as "no id" instead of failing the whole post.
fn deserialize_lenient_id<'de, D>(deserializer: D) -> Result<Option<usize>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::Number(n) => n.as_u64().and_then(|n| usize::try_from(n).ok()),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

/// WordPress returns an empty list (`[]`) instead of an object if a post or user has no ACF data.
/// Treat anything that isn't an object as "no ACF data".
fn deserialize_acf<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    if value.is_object() {
        T::deserialize(value)
            .map(Some)
            .map_err(serde::de::Error::custom)
    } else {
        Ok(None)
    }
}

/// The coauthors endpoint returns the description as `{"raw": "...", "rendered": "<p>...</p>"}`,
/// other setups as a plain string. Prefers the raw variant.
fn deserialize_coauthor_description<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match value {
        Some(serde_json::Value::String(s)) => Some(s),
        Some(serde_json::Value::Object(o)) => ["raw", "rendered"]
            .iter()
            .find_map(|k| o.get(*k).and_then(|v| v.as_str()))
            .map(str::to_string),
        _ => None,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoAuthor {
    /// WordPress user id, or the post id of a "guest author" (Co-Authors Plus)
    #[serde(default, deserialize_with = "deserialize_lenient_id")]
    pub id: Option<usize>,
    pub display_name: String,
    /// Only present for some Co-Authors Plus setups
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub last_name: Option<String>,
    /// The author's biography, if the API returns one (a plain string or `{"raw", "rendered"}`)
    #[serde(default, deserialize_with = "deserialize_coauthor_description")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Post {
    #[serde(rename = "date_gmt")]
    pub date: chrono::NaiveDateTime,
    pub id: usize,
    pub link: String,
    #[serde(rename = "modified_gmt")]
    pub modified: chrono::NaiveDateTime,
    pub slug: String,
    #[serde(rename = "type")]
    pub post_type: String,
    pub title: RenderedContent,
    pub content: RenderedContent,
    pub excerpt: RenderedContent,
    pub author: usize,
    pub featured_media: usize,
    pub categories: Vec<usize>,
    pub tags: Vec<usize>,
    /// Optionally additional fields from the Advanced Custom Fields Plugin
    #[serde(default, deserialize_with = "deserialize_acf")]
    pub acf: Option<PostAcf>,
    /// Optionally additional co-authors (Co-Authors Plus WP Plugin)
    #[serde(skip)]
    pub coauthors: Option<Vec<CoAuthor>>,
    /// Names of the tags in `tags` as WordPress returns them (html-encoded), see
    /// [`WordpressAPI::add_resolved_tags`]
    #[serde(skip)]
    pub tags_resolved: Option<Vec<String>>,
}

/// Similar to ['Post'] but only with fields that are returned by the WordPress api if context=embed
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct PostPreview {
    pub id: usize,
    pub date: chrono::NaiveDateTime,
    pub link: String,
    pub slug: String,
    #[serde(rename = "type")]
    pub post_type: String,
    pub title: RenderedContent,
    pub author: usize,
    pub excerpt: RenderedContent,
    /// Only requested if posts have to be filtered by category, see [`PREVIEW_FIELDS`].
    #[serde(default, skip_serializing)]
    pub categories: Vec<usize>,
}

/// A WordPress tag, as returned by `/wp/v2/tags`
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Tag {
    pub id: usize,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Category {
    pub id: usize,
    /// The number of posts in this category
    pub count: usize,
    pub description: String,
    pub name: String,
    pub slug: String,
    /// The parent category id
    pub parent: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HierarchicalCategory {
    pub id: usize,
    pub count: usize,
    pub description: String,
    pub name: String,
    pub slug: String,
    pub parent: usize,
    // HashMap with categories (ids as keys)
    pub children: Vec<HierarchicalCategory>,
}

impl From<Category> for HierarchicalCategory {
    fn from(c: Category) -> HierarchicalCategory {
        HierarchicalCategory {
            id: c.id,
            count: c.count,
            description: c.description,
            name: c.name,
            slug: c.slug,
            parent: c.parent,
            children: Vec::new(),
        }
    }
}

impl HierarchicalCategory {
    pub fn add_children(&mut self, categories: &mut Vec<Category>) {
        let mut i = 0;

        loop {
            let category = match categories.get(i) {
                Some(category) => category,
                None => break,
            };
            if category.parent == self.id {
                self.children.push(categories.remove(i).into());
            } else {
                i += 1;
            }
        }

        for child in self.children.iter_mut() {
            child.add_children(categories);
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CategoryTree {
    pub categories: Vec<HierarchicalCategory>,
}

impl From<Vec<Category>> for CategoryTree {
    fn from(mut categories: Vec<Category>) -> Self {
        let mut main_category = HierarchicalCategory {
            id: 0,
            count: 0,
            description: "".to_string(),
            name: "".to_string(),
            slug: "".to_string(),
            parent: 0,
            children: Vec::new(),
        };

        main_category.add_children(&mut categories);

        CategoryTree {
            categories: main_category.children,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_import_single_post() {
        let api = WordpressAPI::new("verfassungsblog.de".to_string()).unwrap();
        let post = api.get_post(79100).await.unwrap();
        println!("{:?}", post);
    }

    #[tokio::test]
    #[ignore = "live request to verfassungsblog.de; exceeds the 10s client timeout when the site is slow"]
    async fn test_import_posts() {
        let api = WordpressAPI::new("verfassungsblog.de".to_string()).unwrap();
        let posts = api
            .get_posts(
                WordpressAPIContext::default(),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(5),
            )
            .await
            .unwrap();
        println!("{:?}", posts);
    }

    #[tokio::test]
    #[ignore = "live request to verfassungsblog.de; exceeds the 10s client timeout when the site is slow"]
    async fn test_get_category_tree() {
        let api = WordpressAPI::new("verfassungsblog.de".to_string()).unwrap();
        let categories = api.get_category_tree().await.unwrap();
        println!("Category tree: {:?}", categories.categories);
    }
}

#[cfg(test)]
mod category_query_tests {
    use super::*;

    #[test]
    fn small_filters_fit_into_one_query() {
        let plan = CategoryQueryPlan::new(100, &[1, 2, 3], &[4, 5]);
        assert_eq!(
            plan,
            CategoryQueryPlan {
                include_chunks: vec!["1,2,3".to_string()],
                exclude_in_query: true,
            }
        );
    }

    #[test]
    fn without_included_categories_no_chunk_is_made() {
        let plan = CategoryQueryPlan::new(100, &[], &[4]);
        assert!(plan.include_chunks.is_empty());
        assert!(plan.exclude_in_query);
    }

    #[test]
    fn too_many_excluded_categories_never_produce_an_empty_include_chunk() {
        let exclude: Vec<usize> = (100_000..100_400).collect();
        let plan = CategoryQueryPlan::new(100, &[1, 2, 3], &exclude);
        assert!(!plan.exclude_in_query);
        assert_eq!(plan.include_chunks, vec!["1,2,3".to_string()]);
    }

    /// Builds the request URL like reqwest's `.query()` does, url-encoding the values.
    fn request_url(base_query: &[(String, String)], extra: &[(&str, &str)]) -> String {
        let mut query = base_query.to_vec();
        query.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        query.push(("page".to_string(), "99999".to_string()));
        reqwest::Url::parse_with_params("https://example.org/wp-json/wp/v2/posts", &query)
            .unwrap()
            .to_string()
    }

    fn base_query() -> Vec<(String, String)> {
        vec![
            ("context".to_string(), "view".to_string()),
            ("per_page".to_string(), "100".to_string()),
            ("after".to_string(), "2024-01-01 00:00:00".to_string()),
        ]
    }

    fn base_url_length() -> usize {
        "https://example.org/wp-json/wp/v2/posts".len()
            + query_length(&base_query())
            + PAGE_PARAM_LENGTH
    }

    #[test]
    fn included_categories_are_split_without_exceeding_the_limit() {
        let include: Vec<usize> = (100_000..100_600).collect();
        let plan = CategoryQueryPlan::new(base_url_length(), &include, &[]);
        assert!(plan.include_chunks.len() > 1);
        assert!(plan.include_chunks.iter().all(|c| !c.is_empty()));
        for chunk in &plan.include_chunks {
            let url = request_url(&base_query(), &[("categories", chunk)]);
            assert!(
                url.len() <= MAX_URL_LENGTH,
                "{} > {}",
                url.len(),
                MAX_URL_LENGTH
            );
        }
        let rejoined: Vec<usize> = plan
            .include_chunks
            .iter()
            .flat_map(|c| c.split(','))
            .map(|id| id.parse().unwrap())
            .collect();
        assert_eq!(rejoined, include);
    }

    #[test]
    fn included_and_excluded_categories_together_stay_within_the_limit() {
        let include: Vec<usize> = (100_000..100_300).collect();
        let exclude: Vec<usize> = (200_000..200_150).collect();
        let plan = CategoryQueryPlan::new(base_url_length(), &include, &exclude);
        assert!(plan.exclude_in_query);
        let exclude = join_ids(&exclude);
        for chunk in &plan.include_chunks {
            let url = request_url(
                &base_query(),
                &[("categories_exclude", &exclude), ("categories", chunk)],
            );
            assert!(
                url.len() <= MAX_URL_LENGTH,
                "{} > {}",
                url.len(),
                MAX_URL_LENGTH
            );
        }
    }

    #[test]
    fn id_param_length_matches_the_encoded_parameter() {
        let ids = [1, 22, 333];
        let url = request_url(&[], &[("categories", &join_ids(&ids))]);
        let without = request_url(&[], &[]);
        assert_eq!(
            url.len() - without.len(),
            id_param_length("categories", &ids)
        );
    }

    #[test]
    fn base_url_over_the_limit_does_not_underflow() {
        let plan = CategoryQueryPlan::new(MAX_URL_LENGTH + 500, &[1, 2], &[3]);
        assert!(!plan.exclude_in_query);
        assert_eq!(plan.include_chunks, vec!["1".to_string(), "2".to_string()]);
    }

    fn preview(id: usize) -> PostPreview {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "date": "2024-01-01T00:00:00",
            "link": "",
            "slug": "",
            "type": "post",
            "title": {"rendered": ""},
            "author": 1,
            "excerpt": {"rendered": ""},
        }))
        .unwrap()
    }

    #[test]
    fn duplicate_and_excluded_posts_are_dropped() {
        let mut seen = HashSet::new();
        let excluded: HashSet<usize> = [7].into_iter().collect();

        let mut first = vec![preview(1), preview(2)];
        first[1].categories = vec![3, 7];
        keep_new_posts(&mut first, &mut seen, Some(&excluded));
        assert_eq!(first.iter().map(|p| p.id).collect::<Vec<_>>(), vec![1]);

        let mut second = vec![preview(1), preview(3), preview(3)];
        keep_new_posts(&mut second, &mut seen, Some(&excluded));
        assert_eq!(second.iter().map(|p| p.id).collect::<Vec<_>>(), vec![3]);
    }
}

#[cfg(test)]
mod acf_tests {
    use super::PostAcf;

    #[test]
    fn tolerates_unexpected_acf_field_types() {
        let acf: PostAcf = serde_json::from_str(
            r#"{"doi": false, "subheadline": "Sub", "copyright": [], "crossref_doi": 12, "extra": {"a": 1}}"#,
        )
        .unwrap();
        assert_eq!(acf.doi, None);
        assert_eq!(acf.subheadline.as_deref(), Some("Sub"));
        assert_eq!(acf.copyright, None);
        assert_eq!(acf.crossref_doi.as_deref(), Some("12"));
        assert!(acf.other.contains_key("extra"));
        assert!(serde_json::from_str::<PostAcf>("{}").is_ok());
    }
}

#[cfg(test)]
mod user_bio_tests {
    use super::{WordpressUser, WpAuthorIds, WpBio};

    #[test]
    fn coauthor_tolerates_unexpected_id_types() {
        for id in ["5", r#""5""#, "-1", "null", "false", r#""x""#] {
            let c: super::CoAuthor =
                serde_json::from_str(&format!(r#"{{"id": {id}, "display_name": "A B"}}"#)).unwrap();
            assert_eq!(
                c.id,
                if id == "5" || id == r#""5""# {
                    Some(5)
                } else {
                    None
                }
            );
        }
        assert_eq!(
            serde_json::from_str::<super::CoAuthor>(r#"{"display_name": "A"}"#)
                .unwrap()
                .id,
            None
        );
    }

    #[test]
    fn user_parses_with_any_combination_of_acf_fields() {
        for acf in [
            r#""acf": {"orcid": "0000-0001-9810-6222"}"#,
            r#""acf": {"author_fr_desc": "Bio"}"#,
            r#""acf": {"gndid": false, "rorid": [], "orcid": null, "author_eng_desc": 5}"#,
            r#""acf": null"#,
            r#""acf": false"#,
            r#""acf": {}"#,
            r#""acf": []"#,
            r#""other": 1"#,
        ] {
            let user: WordpressUser = serde_json::from_str(&format!(
                r#"{{"id": 1, "name": "A B", "url": "", "description": "", "link": "", "slug": "a-b", {acf}}}"#
            ))
            .unwrap_or_else(|e| panic!("{acf}: {e}"));
            let _ = user.identifiers();
            let _ = user.biographies();
        }
        let user: WordpressUser = serde_json::from_str(
            r#"{"id": 1, "name": "A B", "url": "", "description": "", "link": "", "slug": "a-b", "acf": {"gndid": false, "rorid": [], "orcid": null}}"#,
        )
        .unwrap();
        assert_eq!(user.identifiers(), WpAuthorIds::default());
        assert!(user.biographies().is_empty());
    }

    #[test]
    fn reads_identifiers_from_user_acf() {
        let user: WordpressUser = serde_json::from_str(
            r#"{"id": 1, "name": "A B", "url": "", "description": "", "link": "", "slug": "a-b",
                "acf": {"orcid": "https://orcid.org/0000-0001-9810-6222/ ", "gndid": "", "rorid": "00rcxh774"}}"#,
        )
        .unwrap();
        assert_eq!(
            user.identifiers(),
            WpAuthorIds {
                orcid: Some("0000-0001-9810-6222".to_string()),
                gnd: None,
                ror: Some("00rcxh774".to_string()),
            }
        );
    }

    #[test]
    fn reads_translated_bios_from_user_acf() {
        let user: WordpressUser = serde_json::from_str(
            r#"{"id": 2696, "name": "Joana Setzer", "url": "", "description": "", "link": "", "slug": "joana-setzer",
                "acf": {"orcid": "", "author_eng_desc": "<p>Joana is a professor.</p>\n", "author_fr_desc": false}}"#,
        )
        .unwrap();
        assert_eq!(
            user.biographies(),
            vec![WpBio {
                lang: Some(language::Language::EnUs),
                html: "<p>Joana is a professor.</p>\n".to_string()
            }]
        );
    }

    #[test]
    fn tolerates_missing_or_empty_user_acf() {
        for acf in [r#""acf": []"#, r#""acf": {}"#, r#""x": 1"#] {
            let user: WordpressUser = serde_json::from_str(&format!(
                r#"{{"id": 1, "name": "A B", "url": "", "description": "Bio", "link": "", "slug": "a-b", {acf}}}"#
            ))
            .unwrap();
            let bios = user.biographies();
            assert_eq!(bios.len(), 1);
            assert_eq!(bios[0].lang, None);
        }
    }
}
