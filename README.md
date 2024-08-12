<div align="center">
# Floundr: Container Registry + TUI Client

<img src="config/logo.png" alt="logo" width="140" />
</div>

**Floundr** is a _WIP_ Docker container registry written in Rust with Axum, and a TUI client built with [Ratatui](https://github.com/ratatui-org/ratatui)
The goal is a registry designed to be simple, efficient, and easy to manage, aiming to be fully compliant with
the OCI distribution spec.

## Current State/Existing Features

- **Authentication:** `docker login` is supported, with basic and bearer authentication.
- **User Management**: Role-based access control and user management in the TUI client.
- **Basic Docker Push/Pull:** basic `docker push | pull` commands are supported.
- **TUI Client:** _WIP_ Manage the registry through a terminal-based interface built with Ratatui.
- **Storage Backend:** Local storage is currently supported (Tokio async I/O)

### Roadmap | TODO

- **Complete Specification Compliance**: Floundr aims to be fully compliant with the OCI distribution spec.
- **S3 Storage Driver**: Add support for S3 storage backend.
- **Garbage Collection**: Run garbage collection to remove ref-counted unused layers on a schedule.

## Installation

To get started with Floundr, you'll need to have Rust installed on your system. You can install Rust using [rustup](https://rustup.rs/).

1. Clone the repository:

   ```sh
   git clone https://github.com/PThorpe92/floundr.git
   cd floundr
   ```

2. Build the project:

   ```sh
   cargo build --release
   ```

3. Set the `FLOUNDR_HOME` & `DATABASE_URL` environment variables

   ```sh
   export FLOUNDR_HOME=/path/to/your/storage
   export DATABASE_URL=/path/to/your/db
   ```

4. Run the server and create a new repository

```sh
./target/release/floundr --new-repo <my-repo> --public <true> --new-user <email> --password <password>
```

6. Compile and Run the TUI client

```sh
cd tui_client && cargo run --release
```

7. The client will write a floundr.yml file to the value of your `FLOUNDR_HOME` environment variable
   It will have the default email + password, which you can swap out for the ones you created.
   Optionally, you can also create a new API key with the `--secret <file>` flag and it will write the key to
   the file you specify. Be aware that API keys hold full scope to all repositories, and if logging in with
   `docker login`, it will only request the scope needed for the operation.

Run --help for all options

```sh
Usage: floundr [OPTIONS]

Options:
  -p, --port <PORT>
          [default: 8080]
      --storage-path <STORAGE_PATH>

      --container-home-dir <CONTAINER_HOME_DIR>

      --db-path <DB_PATH>

      --migrate-fresh

      --new-repo <NEW_REPO>
          Create a new repository with a given name
      --public <PUBLIC>
          whether the new repository is public [possible values: true, false]
      --email <EMAIL>
          email for new user
      --password <PASSWORD>
          new user password
      --driver <DRIVER>
          [default: local] [possible values: local] # (todo s3)
      --debug
          Enable debug mode
      --secret <SECRET>
          generate new registry secret and write to file
  -h, --help
          Print help
  -V, --version
          Print version
```

### TUI Client

The TUI client provides a straightforward interface for managing your images and repositories.
Manage users, repositories and API keys with vim keybindings.

<img src="config/tui_client.png" alt="TUI Client" width="600"/>

<img src="config/tui_client2.png" alt="TUI Client" width="600"/>

<img src="config/tui_client3.png" alt="TUI Client" width="600"/>

## Contributing

Contributions are welcome! Lots of low hanging fruit. Please feel free to submit issues, feature requests, or pull requests.

## License

Floundr is released under the MIT License. See the [LICENSE](LICENSE) file for more details.

## Why?

Part RWIIR (re-write it in rust) syndrome, part any reason to use [ratatui](https://github.com/ratatui-org/ratatui)

For any questions, please open an issue or reach out to [preston@pthorpe92.dev](mailto:preston@pthorpe92.dev).
