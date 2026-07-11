#[tokio::main]
async fn main() -> Result<(), model_port::AppError> {
    model_port::run(std::env::args().skip(1).collect()).await
}
