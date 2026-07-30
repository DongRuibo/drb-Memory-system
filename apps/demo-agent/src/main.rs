use demo_agent::{AnswerTurn, DemoReport};

#[tokio::main]
async fn main() {
    println!("LazyTT Demo Agent v1");
    println!(
        "Running memory capture -> new Agent attempt -> Kimi -> forget -> new Agent attempt..."
    );

    match demo_agent::run_from_env().await {
        Ok(report) => print_report(&report),
        Err(error) => {
            eprintln!("Demo Agent v1 FAILED: {error}");
            std::process::exit(1);
        }
    }
}

fn print_report(report: &DemoReport) {
    println!("\n[1/6] Canonical memory accepted");
    println!(
        "operation_id={} memory_record_id={} head_revision={} canonical={} projection={}",
        report.remember_accepted.operation_id,
        report.remember_accepted.memory_record_id,
        report.remember_accepted.head_revision,
        report.remember_accepted.canonical_status,
        report.remember_accepted.projection_status
    );

    println!("\n[2/6] Retrieval projection ready");
    println!(
        "operation_id={} projection={} provider_object_id={} attempts={}",
        report.remember_ready.operation_id,
        report.remember_ready.projection_status,
        report
            .remember_ready
            .provider_object_id
            .as_deref()
            .unwrap_or("unavailable"),
        report.remember_ready.attempts
    );

    println!("\n[3/6] New Agent attempt answered with canonical memory");
    print_answer_turn(&report.with_memory);

    println!("\n[4/6] Canonical Forget accepted");
    println!(
        "operation_id={} memory_record_id={} head_revision={} canonical={} projection={}",
        report.forget_accepted.operation_id,
        report.forget_accepted.memory_record_id,
        report.forget_accepted.head_revision,
        report.forget_accepted.canonical_status,
        report.forget_accepted.projection_status
    );

    println!("\n[5/6] New Agent attempt answered after immediate read denial");
    print_answer_turn(&report.without_memory);

    println!("\n[6/6] Provider projection deleted");
    println!(
        "operation_id={} projection={} attempts={}",
        report.delete_completed.operation_id,
        report.delete_completed.projection_status,
        report.delete_completed.attempts
    );

    println!(
        "\nDemo Agent v1 PASSED: the first fresh attempt used memory {}, the second used none.",
        report.remember_accepted.memory_record_id
    );
}

fn print_answer_turn(turn: &AnswerTurn) {
    println!(
        "task_id={} attempt_id={} context_pack_id={} outcome_receipt_id={} memory_context_count={} used_memory_ids={:?}",
        turn.task_id,
        turn.attempt_id,
        turn.context_pack_id,
        turn.outcome_receipt_id,
        turn.context_memory_ids.len(),
        turn.context_memory_ids
    );
    println!(
        "model={} request_id={} usage(prompt={:?}, completion={:?}, total={:?})",
        turn.model,
        turn.request_id.as_deref().unwrap_or("unavailable"),
        turn.usage.prompt_tokens,
        turn.usage.completion_tokens,
        turn.usage.total_tokens
    );
    println!("--- answer ---\n{}\n--- end answer ---", turn.answer);
}
