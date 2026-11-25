fn main() -> anyhow::Result<()> {
    if workshop::provider::gemini::is_bridge_process() {
        return workshop::provider::gemini::serve_stdio();
    }
    workshop::cli::execute(std::env::args_os())
}
