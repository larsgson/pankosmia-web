# pankosmia_docker

Hosted Pankosmia server: GitHub-backed multi-language
Bible-translation collaboration. A Rocket-based Rust binary that
forks, pushes, and opens GitHub pull requests on behalf of
translators editing in a browser, so translators never visit
GitHub directly. Audio is offloaded to object storage.

```rust
use pankosmia_docker::rocket;
```

See the project's [`README`](https://github.com/larsgson/pankosmia-docker)
for documentation, deployment guidance, and roadmap.

## Attribution

Forked from
[`pankosmia/pankosmia-web`](https://github.com/pankosmia/pankosmia-web),
MIT-licensed. The fork relationship has been severed on GitHub;
this project is no longer affiliated with the Pankosmia
organization. Endpoint URLs and on-disk content layout retain
backwards compatibility with `pankosmia/pankosmia-web` v0.14.x
clients where practical.
