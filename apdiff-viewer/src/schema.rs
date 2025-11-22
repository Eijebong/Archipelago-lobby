diesel::table! {
    fuzz_results (id) {
        id -> Int8,
        world_name -> Varchar,
        version -> Varchar,
        checksum -> Varchar,
        total -> Int4,
        success -> Int4,
        failure -> Int4,
        timeout -> Int4,
        ignored -> Int4,
        task_id -> Varchar,
        pr_number -> Nullable<Int4>,
        extra_args -> Nullable<Varchar>,
        recorded_at -> Timestamptz,
    }
}
