use actix_web::middleware;
use actix_web::{App, HttpResponse, HttpServer, Result, middleware::Logger, web};
use handlebars::{DirectorySourceOptions, Handlebars};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use tokio::fs;

type LoadDirectoryRecursiveResult = Result<Map<String, Value>, Box<dyn std::error::Error>>;

fn load_directory_recursive(
    dir_path: &Path,
) -> Pin<Box<dyn Future<Output = LoadDirectoryRecursiveResult> + '_>> {
    Box::pin(async move {
        let mut data = Map::new();

        if !dir_path.exists() {
            return Ok(data);
        }

        let mut entries = fs::read_dir(dir_path).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let metadata = entry.metadata().await?;

            if metadata.is_dir() {
                // Recursively load subdirectory
                if let Some(dir_name) = path.file_name() {
                    let key = dir_name.to_string_lossy().to_string();
                    match load_directory_recursive(&path).await {
                        Ok(subdir_data) => {
                            data.insert(key, Value::Object(subdir_data));
                            println!("Loaded directory: {}", path.display());
                        }
                        Err(e) => {
                            eprintln!("Failed to load directory {}: {}", path.display(), e);
                        }
                    }
                }
            } else if metadata.is_file() {
                // Process files
                if let Some(extension) = path.extension()
                    && let Some(file_stem) = path.file_stem()
                {
                    let key = file_stem.to_string_lossy().to_string();

                    match extension.to_string_lossy().as_ref() {
                        "json" => match fs::read_to_string(&path).await {
                            Ok(content) => match serde_json::from_str::<Value>(&content) {
                                Ok(json_value) => {
                                    data.insert(key, json_value);
                                    println!("Loaded JSON file: {}", path.display());
                                }
                                Err(e) => {
                                    eprintln!(
                                        "Failed to parse JSON file {}: {}",
                                        path.display(),
                                        e
                                    );
                                }
                            },
                            Err(e) => {
                                eprintln!("Failed to read file {}: {}", path.display(), e);
                            }
                        },
                        "txt" => match fs::read_to_string(&path).await {
                            Ok(content) => {
                                data.insert(key, Value::String(content));
                                println!("Loaded text file: {}", path.display());
                            }
                            Err(e) => {
                                eprintln!("Failed to read file {}: {}", path.display(), e);
                            }
                        },
                        _ => {
                            // Ignore files with other extensions
                        }
                    }
                }
            }
        }

        Ok(data)
    })
}

async fn load_data_files() -> Result<HashMap<String, Value>, Box<dyn std::error::Error>> {
    let data_dir = Path::new("data");

    if !data_dir.exists() {
        println!("Data directory 'data/' does not exist, creating empty context");
        return Ok(HashMap::new());
    }

    let data_map = load_directory_recursive(data_dir).await?;

    // Convert Map<String, Value> to HashMap<String, Value>
    let mut data = HashMap::new();
    for (key, value) in data_map {
        data.insert(key, value);
    }

    Ok(data)
}

async fn render_page(
    path: web::Path<String>,
    _hb: web::Data<Handlebars<'_>>,
) -> Result<HttpResponse> {
    // Get page name, default to "index" if None
    let page = path.into_inner();
    let page = match page.is_empty() {
        true => "index".to_string(),
        false => page,
    };

    // Create a fresh Handlebars instance for this request
    let mut handlebars = Handlebars::new();
    handlebars.set_dev_mode(true);

    // Register all templates from the templates directory
    if let Err(e) =
        handlebars.register_templates_directory("templates", DirectorySourceOptions::default())
    {
        eprintln!("Failed to register templates: {}", e);
        return Ok(HttpResponse::InternalServerError().body("Failed to load templates"));
    }

    // Load all data files
    let data = match load_data_files().await {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Failed to load data files: {}", e);
            return Ok(HttpResponse::InternalServerError().body("Failed to load data files"));
        }
    };

    // Convert HashMap to serde_json::Map for template context
    let mut context = Map::new();
    for (key, value) in data {
        context.insert(key, value);
    }

    // Template path
    let template_name = format!("pages/{}", page);

    // Render the template
    match handlebars.render(&template_name, &context) {
        Ok(rendered) => Ok(HttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .body(rendered)),
        Err(e) => {
            eprintln!("Template rendering error for '{}': {}", template_name, e);
            Ok(HttpResponse::NotFound()
                .body(format!("Template '{}' not found or rendering failed", page)))
        }
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // Initialize logger
    env_logger::init();

    // Create a dummy Handlebars instance (we'll create fresh ones per request)
    let handlebars = Handlebars::new();

    println!("Server starting on http://127.0.0.1:8080");
    println!("Templates directory: ./templates/");
    println!("Data directory: ./data/");
    println!("Auto-detecting new templates and data files on each request");

    // Create and run the HTTP server
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(handlebars.clone()))
            .wrap(Logger::default())
            .wrap(middleware::NormalizePath::trim())
            .route("/{page}", web::get().to(render_page))
            .service(web::Redirect::new("/", "/index").permanent())
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
