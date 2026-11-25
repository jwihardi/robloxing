fn main() -> anyhow::Result<()> {
    workshop::cli::execute(std::env::args_os())
}
