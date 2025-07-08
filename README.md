# [decomp.dev](https://decomp.dev)

Decompilation progress website & GitHub bot.

For more information, see the [wiki](https://wiki.decomp.dev/tools/decomp-dev).

## Backend Setup

1. Install Rust via [rustup](https://rustup.rs/):
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```
2. Clone the repository:
   ```bash
   git clone https://github.com/encounter/decomp.dev.git
   cd decomp.dev
   ```
3. Copy `config.example.yml` to `config.yml`:
   ```bash
   cp config.example.yml config.yml
   ```
4. [Create a new GitHub personal access token](https://github.com/settings/tokens/new?description=decomp.dev&scopes=workflow,write:discussion) with scopes `workflow`, `write:discussion`. Set it in `config.yml`:
    ```yaml
    github:
      token: ghp_abcd1234abcd1234abcd1234abcd1234abcd
    ```
5. Install [bacon](https://dystroy.org/bacon/):
   ```bash
   cargo install --locked bacon
   ```
6. Run the backend with automatic rebuild on changes:
   ```bash
   bacon
   ```

When modifying Rust code, it will take a few seconds to build and restart the server.
Once the server updates, the browser page will automatically reload.

## Frontend Setup

1. Install dependencies:
   ```bash
   npm install
   ```
2. Start the frontend development server:
   ```bash
   npm start
   ```

When the frontend development server is running, changes to files under `css/`, `js/` and `assets/` will live reload in the browser.

## Testing

When running both the backend and frontend, the site will be accessible at http://localhost:3000.

By default, `server.dev_mode` in `config.yml` is enabled, allowing any visitor to log in as a superuser without authentication.

To add projects to the site, visit http://localhost:3000/manage/new, using an existing project from [decomp.dev](https://decomp.dev/) as reference:

![New project page](/docs/manage_new.png)
