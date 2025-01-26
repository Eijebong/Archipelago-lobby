# Archipelago world manager

This repository contains a library as well as tools to manage archipelago
worlds based on an index file.

## Index file

The index file is made of a base file and then `world` ones.

### Base file

The base file should be named `index.toml` and contains information to be
displayed about the index as well as the base version for archipelago.


```toml
archipelago_repo = "https://github.com/ArchipelagoMW/Archipelago.git"
archipelago_version = "0.5.0"
index_homepage = "https://github.com/Eijebong/Archipelago-index"
index_dir = "index"
```

The `archipelago_repo` and `archipelago_version` will be used to download
supported worlds so it's important that they point to a proper git repository
and a proper git ref.

The `homepage` is just a way for users of the index to trace it back to
something.

It points to an `index_dir` directory containing different worlds files.

### World file

Every world should be contained in its own file, named `{world_name}.toml`. The
world name **must** match the apworld name.

For example, for pokemon crystal, you'd have an `index/pokemon_crystal.toml` file with the following content:
```toml
name = "Pokemon Crystal"
home = "https://discord.com/channels/731205301247803413/1057476528419647572"

[versions]
"2.0.0" = { "url" = "https://github.com/AliceMousie/Archipelago/releases/download/2.0.0/pokemon_crystal.apworld" }
"2.1.0" = { "url" = "https://github.com/AliceMousie/Archipelago/releases/download/2.1.0/pokemon_crystal.apworld" }
```

- `name`: The visible name for the APWorld, this could be anything but should probably be the title of the game
- `home`: A URL to where people can find information about the apworld. This can be a github repo, a discord thread link...

Note that the versions must be semver compliant.

#### Templating URLs

Because URLs are usually the same and the version is the only change, you can
have a `default_url` for a world containing `{{version}}`.
The toml above for pokemon crytsal would thus become:

```toml
name = "Pokemon Crystal"
home = "https://discord.com/channels/731205301247803413/1057476528419647572"
default_url = "https://github.com/AliceMousie/Archipelago/releases/download/{{version}}/pokemon_crystal.apworld"
[versions]
"2.0.0" = {}
"2.1.0" = {}
```

#### Default Versions

You can specify a default version for a world as follows:

```toml
name = "Pokemon Crystal"
home = "https://discord.com/channels/731205301247803413/1057476528419647572"
default_url = "https://github.com/AliceMousie/Archipelago/releases/download/{{version}}/pokemon_crystal.apworld"
default_version = "2.0.0"
[versions]
"2.0.0" = {}
"2.1.0" = {}
```

This instructs any consumer of this world to treat that version as default, if
it is not provided then the latest version will be used.

Additionally, `default_version` can also be:
 - `"latest"`: Uses the latest version.
 - `"latest_supported"`: Uses the latest supported version. Only valid for supported worlds.
 - `"disabled"`: Disables the world by default.
