use std::collections::BTreeSet;
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;

use anyhow::{Context, Result, anyhow};
use chrono::{Datelike, Utc};
use chrono_tz::Tz;
use clap::{Args, Parser, Subcommand};
use nutrition_assistant::ai::OpenAiWeeklyPlanGenerator;
use nutrition_assistant::approval::{approve, is_approved, preview_hash, revoke};
use nutrition_assistant::bls::{BlsProvider, import_bls_xlsx};
use nutrition_assistant::catalog::{CuratedFoodCatalog, FoodDataProvider};
use nutrition_assistant::dry_run::{DryRunPlanner, DryRunPreview};
use nutrition_assistant::feedback::{
    FeedbackScope, FeedbackSignal, FeedbackStore, FeedbackTargetKind, NewFeedback, apply_feedback,
};
use nutrition_assistant::mealie::MealieReadClient;
use nutrition_assistant::models::RunState;
use nutrition_assistant::planning_cycle::next_cycle_start;
use nutrition_assistant::profile::load_profile;
use nutrition_assistant::review::{render_markdown, render_revision_diff};
use nutrition_assistant::revision::RevisionPlanner;
use nutrition_assistant::storage::RunLedger;
use nutrition_assistant::validation::{IssueSeverity, validate_profile_readiness};

#[derive(Parser)]
#[command(
    name = "nutrition-assistant",
    version,
    about = "Deterministic private weekly nutrition planner"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Render a private human-readable review from a dry-run artifact")]
    Review {
        #[arg(long, default_value = "dry-run.local.json")]
        input: PathBuf,
        #[arg(long, default_value = "review.local.md")]
        output: PathBuf,
    },
    #[command(about = "Compare an immutable recipe revision with its parent preview")]
    ReviewDiff {
        #[arg(long)]
        parent: PathBuf,
        #[arg(long)]
        revision: PathBuf,
        #[arg(long, default_value = "revision-review.local.md")]
        output: PathBuf,
    },
    #[command(about = "Replace one recipe while locking all meal slots and other recipes")]
    Refine {
        #[command(flatten)]
        local: LocalArgs,
        #[arg(long, default_value = "dry-run.local.json")]
        input: PathBuf,
        #[arg(long)]
        recipe_id: String,
        #[arg(long)]
        instruction: String,
        #[arg(long, default_value = "revision.local.json")]
        output: PathBuf,
    },
    #[command(about = "Generate and validate without remote writes")]
    DryRun {
        #[command(flatten)]
        storage: StorageArgs,
        #[arg(long)]
        year: i32,
        #[arg(long)]
        week: u32,
    },
    #[command(about = "Generate the next shopping-day cycle without remote writes")]
    NextDryRun {
        #[command(flatten)]
        storage: StorageArgs,
    },
    #[command(about = "Validate all local inputs without network access or secrets")]
    Preflight {
        #[command(flatten)]
        local: LocalArgs,
    },
    #[command(about = "Inspect and verify curated nutrition mappings without modifying them")]
    Catalog {
        #[command(subcommand)]
        command: CatalogCommand,
    },
    #[command(about = "Import and inspect the local BLS nutrition dataset")]
    Bls {
        #[command(subcommand)]
        command: BlsCommand,
    },
    #[command(about = "Store and inspect private planning feedback")]
    Feedback {
        #[command(subcommand)]
        command: FeedbackCommand,
    },
    #[command(about = "Approve, inspect, or revoke an exact private preview artifact")]
    Approval {
        #[command(subcommand)]
        command: ApprovalCommand,
    },
}

#[derive(Subcommand)]
enum ApprovalCommand {
    #[command(about = "Explicitly approve the exact artifact hash for later publishing")]
    Approve {
        #[arg(long, default_value = "dry-run.local.json")]
        input: PathBuf,
        #[arg(long, default_value = "planner.local.sqlite3")]
        database: PathBuf,
    },
    #[command(about = "Check whether the exact artifact hash is currently approved")]
    Status {
        #[arg(long, default_value = "dry-run.local.json")]
        input: PathBuf,
        #[arg(long, default_value = "planner.local.sqlite3")]
        database: PathBuf,
    },
    #[command(about = "Revoke approval before publishing has started")]
    Revoke {
        #[arg(long, default_value = "dry-run.local.json")]
        input: PathBuf,
        #[arg(long, default_value = "planner.local.sqlite3")]
        database: PathBuf,
    },
}

#[derive(Subcommand)]
enum FeedbackCommand {
    #[command(about = "Add an idempotent permanent or cycle-scoped feedback rule")]
    Add {
        #[arg(long)]
        kind: String,
        #[arg(long)]
        signal: String,
        #[arg(long, default_value = "permanent")]
        scope: String,
        #[arg(long)]
        target: String,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        year: Option<i32>,
        #[arg(long)]
        week: Option<u32>,
        #[arg(long, default_value = "planner.local.sqlite3")]
        database: PathBuf,
    },
    #[command(about = "List locally stored feedback without changing it")]
    List {
        #[arg(long, default_value = "planner.local.sqlite3")]
        database: PathBuf,
    },
    #[command(about = "Explicitly deactivate one feedback rule")]
    Deactivate {
        feedback_key: String,
        #[arg(long, default_value = "planner.local.sqlite3")]
        database: PathBuf,
    },
}

#[derive(Subcommand)]
enum BlsCommand {
    #[command(about = "Import an official BLS 4.0 XLSX file idempotently")]
    Import {
        #[arg(long, default_value = "config/BLS_4_0_Daten_2025_DE.xlsx")]
        input: PathBuf,
        #[arg(long, default_value = "planner.local.sqlite3")]
        database: PathBuf,
    },
}

#[derive(Subcommand)]
enum CatalogCommand {
    #[command(about = "Validate the local catalog without network access")]
    Validate {
        #[arg(long, default_value = "config/food_catalog.local.yaml")]
        catalog: PathBuf,
    },
    #[command(about = "Search imported BLS candidates; never changes the catalog")]
    Search {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: u32,
        #[arg(long, default_value = "planner.local.sqlite3")]
        database: PathBuf,
    },
    #[command(about = "Verify every curated source against the imported BLS dataset")]
    Verify {
        #[arg(long, default_value = "config/food_catalog.local.yaml")]
        catalog: PathBuf,
        #[arg(long, default_value = "planner.local.sqlite3")]
        database: PathBuf,
    },
}

#[derive(Debug, Clone, Args)]
struct StorageArgs {
    #[command(flatten)]
    local: LocalArgs,
    #[arg(long, default_value = "dry-run.local.json")]
    output: PathBuf,
    #[arg(long)]
    skip_destination_check: bool,
}

#[derive(Debug, Clone, Args)]
struct LocalArgs {
    #[arg(long, default_value = "config/profile.local.yaml")]
    profile: PathBuf,
    #[arg(long, default_value = "config/food_catalog.local.yaml")]
    catalog: PathBuf,
    #[arg(long, default_value = "config/BLS_4_0_Daten_2025_DE.xlsx")]
    bls_data: PathBuf,
    #[arg(long, default_value = "planner.local.sqlite3")]
    database: PathBuf,
}

fn required_secret(name: &str) -> Result<String> {
    env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow!("required runtime secret is missing: {name}"))
}

async fn run_dry(args: &StorageArgs, iso_year: i32, iso_week: u32) -> Result<DryRunPreview> {
    let mut profile = load_profile(&args.local.profile)?;
    let feedback =
        FeedbackStore::open(&args.local.database)?.active_for_cycle(iso_year, iso_week)?;
    apply_feedback(&mut profile, &feedback);
    let catalog = CuratedFoodCatalog::load(&args.local.catalog)?;
    let generator = OpenAiWeeklyPlanGenerator::new(
        required_secret("OPENAI_API_KEY")?,
        env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-6.2-sol".into()),
    )?;
    let imported = import_bls_xlsx(&args.local.database, &args.local.bls_data)?;
    let provider = BlsProvider::open(&args.local.database, imported.source_version)?;
    let mut ledger = RunLedger::open(&args.local.database)?;
    let mealie = if args.skip_destination_check {
        None
    } else {
        Some(MealieReadClient::new(
            required_secret("MEALIE_URL")?,
            required_secret("MEALIE_API_TOKEN")?,
        )?)
    };
    let mut planner = DryRunPlanner {
        generator: &generator,
        food_provider: &provider,
        catalog: &catalog,
        ledger: &mut ledger,
        mealie: mealie
            .as_ref()
            .map(|v| v as &dyn nutrition_assistant::mealie::MealieReader),
    };
    let preview = planner.run(&profile, iso_year, iso_week).await?;
    std::fs::write(&args.output, serde_json::to_string_pretty(&preview)?)?;
    Ok(preview)
}

#[tokio::main]
async fn main() -> ExitCode {
    match execute().await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(1)
        }
    }
}

async fn execute() -> Result<ExitCode> {
    load_local_env()?;
    let cli = Cli::parse();
    if let Command::Review { input, output } = &cli.command {
        let preview: DryRunPreview = serde_json::from_slice(
            &std::fs::read(input).with_context(|| format!("failed to read {}", input.display()))?,
        )
        .with_context(|| format!("invalid dry-run artifact: {}", input.display()))?;
        std::fs::write(output, render_markdown(&preview))
            .with_context(|| format!("failed to write {}", output.display()))?;
        println!("private review written to {}", output.display());
        return Ok(ExitCode::SUCCESS);
    }
    if let Command::ReviewDiff {
        parent,
        revision,
        output,
    } = &cli.command
    {
        let read_preview = |path: &PathBuf| -> Result<DryRunPreview> {
            serde_json::from_slice(
                &std::fs::read(path)
                    .with_context(|| format!("failed to read {}", path.display()))?,
            )
            .with_context(|| format!("invalid dry-run artifact: {}", path.display()))
        };
        let parent = read_preview(parent)?;
        let revision = read_preview(revision)?;
        std::fs::write(output, render_revision_diff(&parent, &revision))
            .with_context(|| format!("failed to write {}", output.display()))?;
        println!("private revision review written to {}", output.display());
        return Ok(ExitCode::SUCCESS);
    }
    if let Command::Refine {
        local,
        input,
        recipe_id,
        instruction,
        output,
    } = &cli.command
    {
        let parent: DryRunPreview = serde_json::from_slice(
            &std::fs::read(input).with_context(|| format!("failed to read {}", input.display()))?,
        )
        .with_context(|| format!("invalid dry-run artifact: {}", input.display()))?;
        let plan = parent
            .plan
            .as_ref()
            .ok_or_else(|| anyhow!("parent preview has no plan"))?;
        let mut profile = load_profile(&local.profile)?;
        let feedback =
            FeedbackStore::open(&local.database)?.active_for_cycle(plan.iso_year, plan.iso_week)?;
        apply_feedback(&mut profile, &feedback);
        let catalog = CuratedFoodCatalog::load(&local.catalog)?;
        let generator = OpenAiWeeklyPlanGenerator::new(
            required_secret("OPENAI_API_KEY")?,
            env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-6.2-sol".into()),
        )?;
        let imported = import_bls_xlsx(&local.database, &local.bls_data)?;
        let provider = BlsProvider::open(&local.database, imported.source_version)?;
        let mut ledger = RunLedger::open(&local.database)?;
        let mut planner = RevisionPlanner {
            refiner: &generator,
            food_provider: &provider,
            catalog: &catalog,
            ledger: &mut ledger,
        };
        let revision = planner
            .revise(&parent, &profile, recipe_id, instruction)
            .await?;
        std::fs::write(output, serde_json::to_string_pretty(&revision)?)
            .with_context(|| format!("failed to write {}", output.display()))?;
        println!(
            "recipe revision state={} issues={} writes=0 output={}",
            revision.state.as_str(),
            revision.issues.len(),
            output.display()
        );
        return Ok(if revision.state == RunState::Validated {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(2)
        });
    }
    if let Command::Catalog { command } = &cli.command {
        run_catalog_command(command).await?;
        return Ok(ExitCode::SUCCESS);
    }
    if let Command::Bls { command } = &cli.command {
        run_bls_command(command)?;
        return Ok(ExitCode::SUCCESS);
    }
    if let Command::Feedback { command } = &cli.command {
        run_feedback_command(command)?;
        return Ok(ExitCode::SUCCESS);
    }
    if let Command::Approval { command } = &cli.command {
        run_approval_command(command)?;
        return Ok(ExitCode::SUCCESS);
    }
    if let Command::Preflight { local } = &cli.command {
        return run_preflight(local).await;
    }
    let (args, year, week) = match &cli.command {
        Command::DryRun {
            storage,
            year,
            week,
        } => (storage, *year, *week),
        Command::NextDryRun { storage } => {
            let profile = load_profile(&storage.local.profile)?;
            let day = profile
                .planning
                .shopping_day
                .ok_or_else(|| anyhow!("shopping day is required for next-dry-run"))?;
            let timezone = Tz::from_str(&profile.planning.schedule.timezone)
                .context("invalid profile timezone")?;
            let target = next_cycle_start(Utc::now().with_timezone(&timezone).date_naive(), day);
            let iso = target.iso_week();
            (storage, iso.year(), iso.week())
        }
        Command::Catalog { .. } => unreachable!("catalog command returned above"),
        Command::Bls { .. } => unreachable!("BLS command returned above"),
        Command::Preflight { .. } => unreachable!("preflight command returned above"),
        Command::Review { .. } => unreachable!("review command returned above"),
        Command::Feedback { .. } => unreachable!("feedback command returned above"),
        Command::Refine { .. } => unreachable!("refine command returned above"),
        Command::ReviewDiff { .. } => unreachable!("review-diff command returned above"),
        Command::Approval { .. } => unreachable!("approval command returned above"),
    };
    let preview = run_dry(args, year, week).await?;
    let issues = if preview.issues.is_empty() {
        "none".into()
    } else {
        preview
            .issues
            .iter()
            .map(|v| v.code.as_str())
            .collect::<Vec<_>>()
            .join(",")
    };
    println!(
        "dry-run state={} issues={issues} writes=0",
        preview.state.as_str()
    );
    println!("private preview written to {}", args.output.display());
    Ok(if preview.state == RunState::Validated {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    })
}

fn read_preview(path: &PathBuf) -> Result<DryRunPreview> {
    serde_json::from_slice(
        &std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?,
    )
    .with_context(|| format!("invalid dry-run artifact: {}", path.display()))
}

fn run_approval_command(command: &ApprovalCommand) -> Result<()> {
    let (input, database) = match command {
        ApprovalCommand::Approve { input, database }
        | ApprovalCommand::Status { input, database }
        | ApprovalCommand::Revoke { input, database } => (input, database),
    };
    let preview = read_preview(input)?;
    let hash = preview_hash(&preview)?;
    let mut ledger = RunLedger::open(database)?;
    match command {
        ApprovalCommand::Approve { .. } => {
            let created = approve(&preview, &mut ledger)?;
            println!(
                "approval active=true changed={created} run_key={} artifact_sha256={hash}",
                preview.run_key
            );
        }
        ApprovalCommand::Status { .. } => {
            println!(
                "approval active={} run_key={} artifact_sha256={hash}",
                is_approved(&preview, &ledger)?,
                preview.run_key
            );
        }
        ApprovalCommand::Revoke { .. } => {
            println!(
                "approval active=false changed={} run_key={} artifact_sha256={hash}",
                revoke(&preview, &mut ledger)?,
                preview.run_key
            );
        }
    }
    Ok(())
}

fn run_feedback_command(command: &FeedbackCommand) -> Result<()> {
    match command {
        FeedbackCommand::Add {
            kind,
            signal,
            scope,
            target,
            reason,
            year,
            week,
            database,
        } => {
            let mut store = FeedbackStore::open(database)?;
            let (record, created) = store.add(&NewFeedback {
                scope: FeedbackScope::from_str(scope)?,
                target_kind: FeedbackTargetKind::from_str(kind)?,
                signal: FeedbackSignal::from_str(signal)?,
                target: target.clone(),
                reason: reason.clone(),
                iso_year: *year,
                iso_week: *week,
            })?;
            println!(
                "feedback created={} key={} scope={} kind={} signal={} target={}",
                created,
                record.feedback_key,
                record.scope.as_str(),
                record.target_kind.as_str(),
                record.signal.as_str(),
                record.target.replace(['\t', '\n'], " ")
            );
        }
        FeedbackCommand::List { database } => {
            let store = FeedbackStore::open(database)?;
            println!("active\tscope\tkind\tsignal\ttarget\tcycle\tkey");
            for record in store.list()? {
                let cycle = match (record.iso_year, record.iso_week) {
                    (Some(year), Some(week)) => format!("{year}-W{week:02}"),
                    _ => "-".into(),
                };
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    record.active,
                    record.scope.as_str(),
                    record.target_kind.as_str(),
                    record.signal.as_str(),
                    record.target.replace(['\t', '\n'], " "),
                    cycle,
                    record.feedback_key
                );
            }
        }
        FeedbackCommand::Deactivate {
            feedback_key,
            database,
        } => {
            let mut store = FeedbackStore::open(database)?;
            println!("feedback deactivated={}", store.deactivate(feedback_key)?);
        }
    }
    Ok(())
}

fn load_local_env() -> Result<()> {
    match dotenvy::dotenv() {
        Ok(_) => Ok(()),
        Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(anyhow!("failed to load local .env: {error}")),
    }
}

async fn run_preflight(args: &LocalArgs) -> Result<ExitCode> {
    let profile = load_profile(&args.profile)?;
    let issues = validate_profile_readiness(&profile);
    let blockers: Vec<_> = issues
        .iter()
        .filter(|issue| issue.severity == IssueSeverity::Blocker)
        .map(|issue| issue.code.as_str())
        .collect();
    if !blockers.is_empty() {
        println!("preflight ready=false blockers={}", blockers.join(","));
        return Ok(ExitCode::from(2));
    }

    let catalog = CuratedFoodCatalog::load(&args.catalog)?;
    let imported = import_bls_xlsx(&args.database, &args.bls_data)?;
    let provider = BlsProvider::open(&args.database, imported.source_version)?;
    let source_ids: BTreeSet<_> = catalog
        .entries()
        .iter()
        .map(|entry| entry.source_id.as_str())
        .collect();
    for source_id in &source_ids {
        provider.get_food(source_id).await?;
    }
    println!(
        "preflight ready=true catalog_entries={} ingredient_names={} nutrition_sources={} profile_issues={}",
        catalog.entries().len(),
        catalog.generation_names().len(),
        source_ids.len(),
        issues.len()
    );
    Ok(ExitCode::SUCCESS)
}

async fn run_catalog_command(command: &CatalogCommand) -> Result<()> {
    match command {
        CatalogCommand::Validate { catalog } => {
            let catalog = CuratedFoodCatalog::load(catalog)?;
            println!(
                "catalog valid entries={} names={} version={}",
                catalog.entries().len(),
                catalog.generation_names().len(),
                catalog.version()
            );
        }
        CatalogCommand::Search {
            query,
            limit,
            database,
        } => {
            if !(1..=50).contains(limit) {
                return Err(anyhow!("catalog search limit must be between 1 and 50"));
            }
            let provider = BlsProvider::open_latest(database)?;
            let candidates = provider.search(query, *limit)?;
            println!("source_id\tdata_type\tdescription");
            for candidate in candidates {
                println!(
                    "{}\t{}\t{}",
                    candidate.source_id,
                    candidate.data_type.replace(['\t', '\n'], " "),
                    candidate.description.replace(['\t', '\n'], " ")
                );
            }
        }
        CatalogCommand::Verify { catalog, database } => {
            let catalog = CuratedFoodCatalog::load(catalog)?;
            let provider = BlsProvider::open_latest(database)?;
            let source_ids: BTreeSet<_> = catalog
                .entries()
                .iter()
                .map(|entry| entry.source_id.as_str())
                .collect();
            println!("source_id\tsource_version\tdata_type\tdescription");
            for source_id in source_ids {
                let record = provider.get_food(source_id).await?;
                println!(
                    "{}\t{}\t{}\t{}",
                    record.source_id,
                    record.source_version.replace(['\t', '\n'], " "),
                    record.data_type.replace(['\t', '\n'], " "),
                    record.description.replace(['\t', '\n'], " ")
                );
            }
        }
    }
    Ok(())
}

fn run_bls_command(command: &BlsCommand) -> Result<()> {
    match command {
        BlsCommand::Import { input, database } => {
            let summary = import_bls_xlsx(database, input)?;
            println!(
                "BLS import version={} records={} skipped_incomplete={} already_present={}",
                summary.source_version,
                summary.imported_records,
                summary.skipped_incomplete_records,
                summary.already_present
            );
        }
    }
    Ok(())
}
