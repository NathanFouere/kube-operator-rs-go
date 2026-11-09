#[allow(unused_imports, unused_variables)]
use actix_web::{
    App, HttpRequest, HttpResponse, HttpServer, Responder, get, middleware, put, web, web::Data,
};
pub use controller::{self, State};

#[get("/health")]
async fn health(_: HttpRequest) -> impl Responder {
    HttpResponse::Ok().json("healthy")
}

#[get("/")]
async fn index(c: Data<State>, _req: HttpRequest) -> impl Responder {
    let d = c.diagnostics().await;
    HttpResponse::Ok().json(&d)
}

// A voir si nécessaire
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("Hello, world!");

    let state = State::default();
    let controller = controller::run(state.clone());

    // Start web server
    let server = HttpServer::new(move || {
        App::new()
            .app_data(Data::new(state.clone()))
            .service(index)
            .service(health)
    })
    .bind("0.0.0.0:8081")?
    .shutdown_timeout(5);
    
    tokio::join!(controller, server.run()).1?;
    Ok(())
}
