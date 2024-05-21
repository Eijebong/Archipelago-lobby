# Archipelago world manager

This repository contains a library as well as tools to manage archipelago
worlds based on an index file.

## Index file

The index file is made of a `common` section and then `worlds`.

### Common section

The common section contains information about the index and the current
archipelago version supported by the index.

```toml
[common]
archipelago_repo = "https://github.com/ArchipelagoMW/Archipelago.git"
archipelago_version = "0.4.6"
homepage = "https://github.com/Eijebong/Archipelago-index"
required_global_files = ["alttp", "AutoSNIClient.py", "AutoWorld.py", "Files.py", "generic", "__init__.py", "LauncherComponents.py"]
```

The `archipelago_repo` and `archipelago_version` will be used to download
supported worlds so it's important that they point to a proper git repository
and a proper git ref.

The `homepage` is just a way for users of the index to trace it back to
something.

`required_global_files` contains a list of files/directories in the `worlds` folder that aren't worlds but are required for archipelago to work.

### Supported worlds

Every supported world should have its own section in the index, looking like this:

```toml
[worlds.pokemon_emerald]
name = "Pokemon Emerald"
supported = "pokemon_emerald"
patches = []
```

The world key should match the apworld name.
- `name`: The visible name for the APWorld, this could be anything but should probably be the title of the game
- `supported`: This key should match the name of the directory of the apworld in the archipelago repository. It should match the world key.
- `patches`: A list of patches to apply to the apworld. Note: this isn't implemented yet
- `dependencies`: A list of files that are required for the apworld to work.
  This should not be used with unsupported worlds. It's only here because some
  worlds (sc2) have 3 folders in the original worlds folder for some reason

### Unsupported worlds

```
[worlds.pokemon_crystal]
name = "Pokemon Crystal"
version = "2.0.0"
url = "https://github.com/AliceMousie/Archipelago/releases/download/2.0.0/pokemon_crystal.apworld"
home = "https://discord.com/channels/731205301247803413/1057476528419647572"
```

As for supported worlds, the world key must match the apworld name.

- `name`: The visible name for the APWorld, this could be anything but should probably be the title of the game
- `version`: The version of the apworld. If it doesn't have any, make one up that would make sense to people
- `url`: The URL where the apworld can be downloaded. This needs to be a direct download URL.
- `homepage`: An URL to where people can find information about the apworld. This can be a github repo, a discord thread link...
- `patches`: A list of patches to apply to the apworld. Note: this isn't implemented yet
