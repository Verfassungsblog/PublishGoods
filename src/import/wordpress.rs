use chrono::NaiveTime;
use futures::future::try_join_all;
use reqwest::header::HeaderValue;
use serde::{Deserialize, Serialize};
use std::fmt::Display;

/// Import from Wordpress API
pub struct WordpressAPI {
    base_url: String,
    username: Option<String>,
    password: Option<String>,
    // Timeout for API requests in milliseconds
    timeout: u64,
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
    pub total_pages: usize,
    pub data: PostDataType,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WordpressUser {
    pub id: usize,
    pub name: String,
    pub url: String,
    pub description: String,
    pub link: String,
    pub slug: String,
}

#[derive(Debug, Default)]
pub enum WordpressAPIContext {
    #[default]
    View,
    Edit,
    Embed,
}

impl Display for WordpressAPIContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let str = match self {
            WordpressAPIContext::View => String::from("view"),
            WordpressAPIContext::Edit => String::from("edit"),
            WordpressAPIContext::Embed => String::from("embed"),
        };
        write!(f, "{}", str)
    }
}

trait HeaderValueAsUsize {
    fn try_into_usize(&self) -> Option<usize>;
}
impl HeaderValueAsUsize for HeaderValue {
    fn try_into_usize(&self) -> Option<usize> {
        if let Ok(val) = self.to_str()
            && let Ok(val) = val.parse::<usize>()
        {
            return Some(val);
        }

        None
    }
}

impl WordpressAPI {
    pub fn new(base_url: String) -> Result<Self, WordpressAPIError> {
        Ok(WordpressAPI {
            base_url,
            username: None,
            password: None,
            timeout: 10000,
            client: WordpressAPI::build_client(10000)?,
        })
    }

    pub fn new_authenticated(
        base_url: String,
        username: String,
        password: String,
    ) -> Result<Self, WordpressAPIError> {
        Ok(WordpressAPI {
            base_url,
            username: Some(username),
            password: Some(password),
            timeout: 10000,
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

    pub async fn get_coauthor(&self, mut post: Post) -> Result<Post, WordpressAPIError> {
        let id = post.id;
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
                        Ok(post)
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

    pub async fn get_posts(
        &self,
        context: WordpressAPIContext,
        page: Option<usize>,
        per_page: Option<usize>,
        search: Option<String>,
        after: Option<chrono::NaiveDate>,
        modified_after: Option<chrono::NaiveDate>,
        before: Option<chrono::NaiveDate>,
        modified_before: Option<chrono::NaiveDate>,
        slug: Option<String>,
        categories: Option<Vec<usize>>,
        categories_exclude: Option<Vec<usize>>,
    ) -> Result<PostData, WordpressAPIError> {
        let url = format!("https://{}/wp-json/wp/v2/posts", self.base_url);

        let client = self.client.clone();
        let request = client.request(reqwest::Method::GET, &url);

        let mut query: Vec<(String, String)> = Vec::new();
        match context {
            WordpressAPIContext::View => {
                query.push(("context".to_string(), "view".to_string()));
            }
            WordpressAPIContext::Edit => {
                error!("edit api context isn't supported yet!");
                return Err(WordpressAPIError::Unsupported(String::from(
                    "edit api context isn't supported yet!",
                )));
            }
            WordpressAPIContext::Embed => {
                query.push(("context".to_string(), "embed".to_string()));
            }
        }
        if let Some(page) = page {
            query.push(("page".to_string(), page.to_string()));
        }
        if let Some(per_page) = per_page {
            query.push(("per_page".to_string(), per_page.to_string()));
        }
        if let Some(search) = search {
            query.push(("search".to_string(), search));
        }
        if let Some(after) = after {
            // Warning: The Wordpress Documentation is wrong: the API expects a DateTime, not a Date!
            query.push((
                "after".to_string(),
                after.and_time(NaiveTime::default()).to_string(),
            ));
        }
        if let Some(modified_after) = modified_after {
            query.push((
                "modified_after".to_string(),
                modified_after.and_time(NaiveTime::default()).to_string(),
            ));
        }
        if let Some(before) = before {
            query.push((
                "before".to_string(),
                before.and_time(NaiveTime::default()).to_string(),
            ));
        }
        if let Some(modified_before) = modified_before {
            query.push((
                "modified_before".to_string(),
                modified_before.and_time(NaiveTime::default()).to_string(),
            ));
        }
        if let Some(slug) = slug {
            query.push(("slug".to_string(), slug));
        }
        if let Some(categories) = categories
            && !categories.is_empty()
        {
            query.push((
                "categories".to_string(),
                categories
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<String>>()
                    .join(","),
            ));
        }
        if let Some(categories_exclude) = categories_exclude
            && !categories_exclude.is_empty()
        {
            query.push((
                "categories_exclude".to_string(),
                categories_exclude
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<String>>()
                    .join(","),
            ));
        }
        debug!("Query is: {:?}", query);
        let request = request.query(&query);
        match request.send().await {
            Ok(response) => {
                if response.status() == 400 {
                    // We will get a bad requests if no posts are found at this page
                    return Err(WordpressAPIError::NotFound);
                }
                let num_of_records: usize = match response.headers().get("X-WP-Total") {
                    Some(num) => match num.try_into_usize() {
                        Some(num) => num,
                        None => {
                            error!("Error parsing X-WP-Total as usize.");
                            debug!("{:?}", response);
                            return Err(WordpressAPIError::UnexpectedResponse);
                        }
                    },
                    None => {
                        error!("X-WP-Total is missing in posts response");
                        debug!("{:?}", response);
                        return Err(WordpressAPIError::UnexpectedResponse);
                    }
                };
                let num_of_pages: usize = match response.headers().get("X-WP-TotalPages") {
                    Some(num) => match num.try_into_usize() {
                        Some(num) => num,
                        None => {
                            error!("Error parsing X-WP-TotalPages as usize.");
                            return Err(WordpressAPIError::UnexpectedResponse);
                        }
                    },
                    None => {
                        error!("X-WP-TotalPages is missing in posts response");
                        return Err(WordpressAPIError::UnexpectedResponse);
                    }
                };
                match context {
                    WordpressAPIContext::View => match response.json::<Vec<Post>>().await {
                        Ok(posts) => {
                            let posts =
                                try_join_all(posts.into_iter().map(|post| self.get_coauthor(post)))
                                    .await?;
                            Ok(PostData {
                                number_of_records: num_of_records,
                                total_pages: num_of_pages,
                                data: PostDataType::FullPosts(posts),
                            })
                        }
                        Err(e) => {
                            error!("Error parsing posts: {}", e);
                            Err(WordpressAPIError::SerdeParsingError)
                        }
                    },
                    WordpressAPIContext::Edit => {
                        error!("edit api context isn't supported yet!");
                        Err(WordpressAPIError::Unsupported(String::from(
                            "edit api context isn't supported yet!",
                        )))
                    }
                    WordpressAPIContext::Embed => match response.json::<Vec<PostPreview>>().await {
                        Ok(posts) => Ok(PostData {
                            number_of_records: num_of_records,
                            total_pages: num_of_pages,
                            data: PostDataType::PostPreviews(posts),
                        }),
                        Err(e) => {
                            error!("Error parsing posts: {}", e);
                            Err(WordpressAPIError::SerdeParsingError)
                        }
                    },
                }
            }
            Err(e) => {
                error!("Error fetching posts: {}", e);
                Err(WordpressAPIError::ReqwestError)
            }
        }
    }

    pub async fn get_post(&self, id: usize) -> Result<Post, WordpressAPIError> {
        let url = format!("https://{}/wp-json/wp/v2/posts/{}", self.base_url, id);
        let client = self.client.clone();
        let response = client.get(&url).send().await;
        match response {
            Ok(response) => {
                let post: Post = match response.json().await {
                    Ok(post) => self.get_coauthor(post).await?,
                    Err(e) => {
                        eprintln!("Error parsing post {}: {}", id, e);
                        return Err(WordpressAPIError::SerdeParsingError);
                    }
                };
                Ok(post)
            }
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

    pub async fn get_category(&self, id: usize) -> Result<Category, WordpressAPIError> {
        let client = self.client.clone();
        let url = format!("https://{}/wp-json/wp/v2/categories/{}", self.base_url, id);
        let response = client.get(&url).send().await;
        match response {
            Ok(response) => {
                let category: Category = match response.json().await {
                    Ok(category) => category,
                    Err(e) => {
                        eprintln!("Error parsing category {}: {}", id, e);
                        return Err(WordpressAPIError::SerdeParsingError);
                    }
                };
                Ok(category)
            }
            Err(e) => {
                eprintln!("Error fetching category {}: {}", id, e);
                Err(WordpressAPIError::ReqwestError)
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PostStatus {
    Publish,
    Future,
    Draft,
    Pending,
    Private,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderedContent {
    pub rendered: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostAcf {
    pub subheadline: Option<String>,
    pub copyright: Option<String>,
    pub doi: Option<String>,
    pub crossref_doi: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoAuthor {
    pub display_name: String,
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
    pub acf: Option<PostAcf>,
    /// Optionally additional co-authors (Co-Authors Plus WP Plugin)
    #[serde(skip)]
    pub coauthors: Option<Vec<CoAuthor>>,
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
                None,
                None,
            )
            .await
            .unwrap();
        println!("{:?}", posts);
    }

    #[tokio::test]
    async fn test_get_category_tree() {
        let api = WordpressAPI::new("verfassungsblog.de".to_string()).unwrap();
        let categories = api.get_category_tree().await.unwrap();
        println!("Category tree: {:?}", categories.categories);
    }
}
