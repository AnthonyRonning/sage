// @generated automatically by Diesel CLI.
// Note: Some types manually adjusted for pgvector and UUID support

use diesel::sql_types::*;
use pgvector::sql_types::Vector;

diesel::table! {
    use diesel::sql_types::*;
    use pgvector::sql_types::Vector;

    agents (id) {
        id -> Uuid,
        name -> Varchar,
        system_prompt -> Text,
        message_ids -> Array<Uuid>,
        llm_config -> Jsonb,
        last_memory_update -> Nullable<Timestamptz>,
        max_context_tokens -> Int4,
        compaction_threshold -> Float4,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use pgvector::sql_types::Vector;

    blocks (id) {
        id -> Uuid,
        agent_id -> Text,
        label -> Varchar,
        description -> Nullable<Text>,
        value -> Text,
        char_limit -> Int4,
        read_only -> Bool,
        version -> Int4,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use pgvector::sql_types::Vector;

    messages (id) {
        id -> Uuid,
        agent_id -> Uuid,
        user_id -> Text,
        role -> Text,
        content -> Text,
        // embedding handled via raw SQL due to pgvector complexity
        sequence_id -> Int8,
        tool_calls -> Nullable<Jsonb>,
        tool_results -> Nullable<Jsonb>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use pgvector::sql_types::Vector;

    passages (id) {
        id -> Uuid,
        agent_id -> Text,
        content -> Text,
        embedding -> Nullable<Vector>,
        tags -> Array<Text>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use pgvector::sql_types::Vector;

    summaries (id) {
        id -> Uuid,
        agent_id -> Uuid,
        from_sequence_id -> Int8,
        to_sequence_id -> Int8,
        content -> Text,
        embedding -> Nullable<Vector>,
        previous_summary_id -> Nullable<Uuid>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;

    user_preferences (id) {
        id -> Uuid,
        agent_id -> Uuid,
        key -> Varchar,
        value -> Text,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;

    scheduled_tasks (id) {
        id -> Uuid,
        agent_id -> Uuid,
        task_type -> Varchar,
        payload -> Jsonb,
        next_run_at -> Timestamptz,
        cron_expression -> Nullable<Varchar>,
        timezone -> Varchar,
        status -> Varchar,
        last_run_at -> Nullable<Timestamptz>,
        run_count -> Int4,
        last_error -> Nullable<Text>,
        description -> Text,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;

    chat_contexts (id) {
        id -> Uuid,
        signal_identifier -> Text,
        context_type -> Varchar,
        display_name -> Nullable<Text>,
        created_at -> Timestamptz,
    }
}

diesel::joinable!(scheduled_tasks -> agents (agent_id));

diesel::allow_tables_to_appear_in_same_query!(
    agents,
    blocks,
    chat_contexts,
    messages,
    passages,
    summaries,
    user_preferences,
    scheduled_tasks,
);
