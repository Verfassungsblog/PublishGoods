use serde::{Deserialize, Serialize};

/// Import from Wordpress API
pub struct WordpressAPI{
    base_url: String,
    username: Option<String>,
    password: Option<String>,
    // Timeout for API requests in milliseconds
    timeout: u64,
}

#[derive(Debug)]
pub enum WordpressAPIError{
    SerdeParsingError,
    ReqwestError,
    InvalidURL,
    NotFound,
}

impl WordpressAPI {
    pub fn new(base_url: String) -> Self {
        WordpressAPI {
            base_url,
            username: None,
            password: None,
            timeout: 10000,
        }
    }

    pub fn new_authenticated(base_url: String, username: String, password: String) -> Self {
        WordpressAPI {
            base_url,
            username: Some(username),
            password: Some(password),
            timeout: 10000,
        }
    }

    fn build_client(&self) -> Result<reqwest::Client, WordpressAPIError>{
        match reqwest::ClientBuilder::new().timeout(std::time::Duration::from_millis(self.timeout)).build() {
            Ok(client) => Ok(client),
            Err(e) => {
                eprintln!("Error building client: {}", e);
                Err(WordpressAPIError::ReqwestError)
            }
        }
    }


    pub async fn get_posts(&self, page: Option<usize>, per_page: Option<usize>, search: Option<String>, after: Option<chrono::NaiveDateTime>, modified_after: Option<chrono::NaiveDateTime>, slug: Option<String>, categories: Option<Vec<usize>>, categories_exclude: Option<Vec<usize>>) -> Result<Vec<Post>, WordpressAPIError>{
        let url = format!("https://{}/wp-json/wp/v2/posts", self.base_url);

        let client = self.build_client()?;
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
        if let Some(after) = after {
            query.push(("after".to_string(), after.to_string()));
        }
        if let Some(modified_after) = modified_after {
            query.push(("modified_after".to_string(), modified_after.to_string()));
        }
        if let Some(slug) = slug {
            query.push(("slug".to_string(), slug));
        }
        if let Some(categories) = categories {
            query.push(("categories".to_string(), categories.iter().map(|i| i.to_string()).collect::<Vec<String>>().join(",")));
        }
        if let Some(categories_exclude) = categories_exclude {
            query.push(("categories_exclude".to_string(), categories_exclude.iter().map(|i| i.to_string()).collect::<Vec<String>>().join(",")));
        }
        println!("Query is: {:?}", query);
        let request = request.query(&query);
        match request.send().await{
            Ok(response) => {
                if response.status() == 400 { // We will get a bad requests if no posts are found at this page
                    return Ok(vec![]);
                }
                match response.json::<Vec<Post>>().await {
                    Ok(posts) => Ok(posts),
                    Err(e) => {
                        eprintln!("Error parsing posts: {}", e);
                        Err(WordpressAPIError::SerdeParsingError)
                    }
                }
            },
            Err(e) => {
                eprintln!("Error fetching posts: {}", e);
                return Err(WordpressAPIError::ReqwestError);
            }
        }
    }

    pub async fn get_post(&self, id: usize) -> Result<Post, WordpressAPIError> {
        let url = format!("https://{}/wp-json/wp/v2/posts/{}", self.base_url, id);
        let client = self.build_client()?;
        let response = client.get(&url).send().await;
        match response {
            Ok(response) => {
                let post: Post = match  response.json().await{
                    Ok(post) => post,
                    Err(e) => {
                        eprintln!("Error parsing post {}: {}", id, e);
                        return Err(WordpressAPIError::SerdeParsingError);
                    }
                };
                Ok(post)
            },
            Err(e) => {
                eprintln!("Error fetching post {}: {}", id, e);
                Err(WordpressAPIError::ReqwestError)
            }
        }
    }

    pub async fn get_categories(&self, page: Option<usize>, per_page: Option<usize>, search: Option<String>, exclude: Option<Vec<usize>>, include: Option<Vec<usize>>, slug: Option<String>, hide_empty: Option<bool>, parent: Option<usize>, post: Option<usize>) -> Result<Vec<Category>, WordpressAPIError>{
        let client = self.build_client()?;
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
            query.push(("exclude".to_string(), exclude.iter().map(|i| i.to_string()).collect::<Vec<String>>().join(",")));
        }
        if let Some(include) = include {
            query.push(("include".to_string(), include.iter().map(|i| i.to_string()).collect::<Vec<String>>().join(",")));
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
        match request.send().await{
            Ok(response) => match response.json::<Vec<Category>>().await{
                Ok(categories) => Ok(categories),
                Err(e) => {
                    eprintln!("Error parsing categories: {}", e);
                    Err(WordpressAPIError::SerdeParsingError)
                }
            },
            Err(e) => {
                eprintln!("Error fetching categories: {}", e);
                return Err(WordpressAPIError::ReqwestError);
            }
        }
    }

    pub async fn get_category(&self, id: usize) -> Result<Category, WordpressAPIError>{
        let client = self.build_client()?;
        let url = format!("https://{}/wp-json/wp/v2/categories/{}", self.base_url, id);
        let response = client.get(&url).send().await;
        match response {
            Ok(response) => {
                let category: Category = match response.json().await{
                    Ok(category) => category,
                    Err(e) => {
                        eprintln!("Error parsing category {}: {}", id, e);
                        return Err(WordpressAPIError::SerdeParsingError);
                    }
                };
                Ok(category)
            },
            Err(e) => {
                eprintln!("Error fetching category {}: {}", id, e);
                Err(WordpressAPIError::ReqwestError)
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PostStatus{
    Publish,
    Future,
    Draft,
    Pending,
    Private
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderedContent{
    pub rendered: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostAcf{
    pub subheadline: Option<String>,
    pub copyright: Option<String>,
    pub doi: Option<String>,
    pub crossref_doi: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoAuthor{
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Post{
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
    pub acf: Option<PostAcf>,
    pub coauthors: Option<Vec<CoAuthor>>
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Category{
    pub id: usize,
    pub count: usize,
    pub description: String,
    pub name: String,
    pub slug: String,
    pub parent: usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_import_single_post() {
        let api = WordpressAPI::new("verfassungsblog.de".to_string());
        let post = api.get_post(79100).await.unwrap();
        println!("{:?}", post);
    }

    #[tokio::test]
    async fn test_import_posts() {
        let api = WordpressAPI::new("verfassungsblog.de".to_string());
        let posts = api.get_posts(None, None, None, None, None, None, None, None).await.unwrap();
        println!("{:?}", posts);
    }
}
