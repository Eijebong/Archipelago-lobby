Archipelago lobby
=================

This project provides a lobby to collect yaml files from players to be able to
host archipelagoes easily.

# Running this project

```
export DATABASE_URL="sqlite:///path/to/your/db.sqlite"
export ADMIN_TOKEN="theadminpassword"
cargo run
```

If you want to run this in a production environment, make sure to set the following too:

```
ROCKET_ENV=production
ROCKET_SECRET_KEY="yoursecretkeyhere" # openssl rand -base64 32
```


