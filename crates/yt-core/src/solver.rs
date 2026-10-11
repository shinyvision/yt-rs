//! Solving the challenges in YouTube's stream URLs.
//!
//! Every stream URL has an `n` parameter, and some have a scrambled
//! signature, that only YouTube's player script turns into what its servers
//! accept; without that they answer 403. The player script changes every few
//! weeks and is obfuscated anew each time, so this runs yt-dlp's challenge
//! solver (bundled in `solver/`, see its README) in an embedded QuickJS: it
//! finds the two functions in the script and calls them.
//!
//! Finding them takes a few seconds (the solver parses the whole 2.5 MB
//! script), so the result, the "preprocessed player", is kept on disk per
//! player version, and its functions stay loaded in the solver's thread.
//! Solving a video's challenges then takes milliseconds.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use rquickjs::{CatchResultExt, Context, Function, Object, Runtime};
use tokio::sync::oneshot;

use crate::error::{Error, Result};

const LIB: &str = include_str!("solver/lib.min.js");
const CORE: &str = include_str!("solver/core.min.js");

/// How long finding the functions in a new player may take, and solving.
const PREPROCESS_LIMIT: Duration = Duration::from_secs(60);
const SOLVE_LIMIT: Duration = Duration::from_secs(5);
const MEMORY_LIMIT: usize = 512 * 1024 * 1024;

/// What a stream lookup needs solved: `n` parameters and signatures.
#[derive(Clone, Debug, Default)]
pub struct Challenges {
    pub n: HashSet<String>,
    pub sig: HashSet<String>,
}

impl Challenges {
    pub fn is_empty(&self) -> bool {
        self.n.is_empty() && self.sig.is_empty()
    }
}

/// Each challenge's answer.
#[derive(Clone, Debug, Default)]
pub struct Solutions {
    pub n: HashMap<String, String>,
    pub sig: HashMap<String, String>,
}

/// The solver, running on its own thread (QuickJS isn't thread-safe).
pub struct Solver {
    jobs: Mutex<mpsc::Sender<Job>>,
    dir: PathBuf,
    loaded: Arc<Mutex<HashSet<String>>>,
}

struct Job {
    player: String,
    /// The player script, if the solver hasn't prepared this version yet.
    code: Option<String>,
    challenges: Challenges,
    reply: oneshot::Sender<Result<Solutions>>,
}

#[yt_export::export(skip = "the stream resolver's own (plugins hook streams.*)")]
impl Solver {
    /// Keeps preprocessed players in `dir`. The JS engine starts with the
    /// first job.
    pub fn new(dir: PathBuf) -> Self {
        let (jobs, rx) = mpsc::channel();
        let loaded = Arc::new(Mutex::new(HashSet::new()));
        let (thread_dir, thread_loaded) = (dir.clone(), loaded.clone());
        std::thread::Builder::new()
            .name("yt-solver".into())
            .spawn(move || run(&rx, &thread_dir, &thread_loaded))
            .expect("could not start the challenge solver's thread");
        Self {
            jobs: Mutex::new(jobs),
            dir,
            loaded,
        }
    }

    /// Whether the solver can handle this player version without its code.
    pub fn knows(&self, player: &str) -> bool {
        self.loaded.lock().unwrap().contains(player)
            || preprocessed_file(&self.dir, player).exists()
    }

    /// Solves the challenges with the functions of the player `player` (its
    /// ID, from the script's URL). Pass its `code` unless [`Solver::knows`]
    /// it. With no challenges, it only gets the player ready.
    pub async fn solve(
        &self,
        player: &str,
        code: Option<String>,
        challenges: Challenges,
    ) -> Result<Solutions> {
        let (reply, answer) = oneshot::channel();
        let job = Job {
            player: player.to_owned(),
            code,
            challenges,
            reply,
        };
        self.jobs
            .lock()
            .unwrap()
            .send(job)
            .map_err(|_| Error::Streams("the challenge solver stopped".into()))?;
        answer
            .await
            .map_err(|_| Error::Streams("the challenge solver stopped".into()))?
    }
}

fn preprocessed_file(dir: &Path, player: &str) -> PathBuf {
    // Player IDs are hex; anything else can't name a file outside `dir`.
    let name: String = player.chars().filter(char::is_ascii_alphanumeric).collect();
    dir.join(format!("{name}.js"))
}

fn run(jobs: &mpsc::Receiver<Job>, dir: &Path, loaded: &Mutex<HashSet<String>>) {
    let mut engine: Option<Engine> = None;
    for job in jobs {
        let result = (|| {
            let engine = match &mut engine {
                Some(engine) => engine,
                None => engine.insert(Engine::new()?),
            };
            engine.load(&job.player, job.code.as_deref(), dir)?;
            loaded.lock().unwrap().insert(job.player.clone());
            engine.solve(&job.challenges)
        })();
        if let Err(e) = &result {
            tracing::warn!(player = job.player, "challenge solver: {e}");
        }
        let _ = job.reply.send(result);
    }
}

/// QuickJS with the solver loaded, and the functions of one player version.
struct Engine {
    runtime: Runtime,
    context: Context,
    /// Until when the running script may run, in milliseconds since `start`.
    deadline: Arc<AtomicU64>,
    start: Instant,
    player: Option<String>,
}

impl Engine {
    fn new() -> Result<Self> {
        let runtime = Runtime::new().map_err(js)?;
        runtime.set_memory_limit(MEMORY_LIMIT);
        let start = Instant::now();
        let deadline = Arc::new(AtomicU64::new(u64::MAX));
        let limit = deadline.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            start.elapsed().as_millis() as u64 > limit.load(Ordering::Relaxed)
        })));
        let context = Context::full(&runtime).map_err(js)?;
        let (lib, core) = scripts();
        context.with(|ctx| {
            ctx.eval::<(), _>(lib.as_str())
                .catch(&ctx)
                .map_err(caught)?;
            ctx.eval::<(), _>("Object.assign(globalThis, lib);")
                .catch(&ctx)
                .map_err(caught)?;
            ctx.eval::<(), _>(core.as_str()).catch(&ctx).map_err(caught)
        })?;
        Ok(Self {
            runtime,
            context,
            deadline,
            start,
            player: None,
        })
    }

    /// Lets the next script run for at most `limit`.
    fn limit(&self, limit: Duration) {
        let until = self.start.elapsed() + limit;
        self.deadline
            .store(until.as_millis() as u64, Ordering::Relaxed);
    }

    /// Loads a player version's functions, preprocessing the player first
    /// if it isn't on disk yet.
    fn load(&mut self, player: &str, code: Option<&str>, dir: &Path) -> Result<()> {
        if self.player.as_deref() == Some(player) {
            return Ok(());
        }
        let file = preprocessed_file(dir, player);
        let preprocessed = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(_) => {
                let code =
                    code.ok_or_else(|| Error::Streams(format!("player {player} isn't prepared")))?;
                let started = Instant::now();
                let text = self.preprocess(code)?;
                tracing::info!(player, took = ?started.elapsed(), "prepared the challenge solver");
                let _ = std::fs::create_dir_all(dir);
                if let Err(e) = std::fs::write(&file, &text) {
                    tracing::warn!("couldn't keep the preprocessed player: {e}");
                }
                text
            }
        };
        self.limit(SOLVE_LIMIT * 4);
        self.context.with(|ctx| {
            ctx.globals()
                .set("__preprocessed", preprocessed)
                .map_err(js)?;
            // What the solver does with a preprocessed player, keeping the
            // functions instead of building them again for every video.
            ctx.eval::<(), _>(
                "globalThis.__solver = (() => {
                   const result = { n: null, sig: null };
                   Function('_result', globalThis.__preprocessed)(result);
                   delete globalThis.__preprocessed;
                   return result;
                 })();",
            )
            .catch(&ctx)
            .map_err(caught)
        })?;
        self.player = Some(player.to_owned());
        self.runtime.run_gc();
        Ok(())
    }

    fn preprocess(&self, code: &str) -> Result<String> {
        self.limit(PREPROCESS_LIMIT);
        let result = self.context.with(|ctx| {
            ctx.globals().set("__player", code).map_err(js)?;
            let result = ctx
                .eval::<String, _>(
                    "(() => {
                       const out = jsc({ type: 'player', player: globalThis.__player, requests: [], output_preprocessed: true });
                       delete globalThis.__player;
                       if (out.type === 'error') throw new Error(out.error);
                       return out.preprocessed_player;
                     })()",
                )
                .catch(&ctx)
                .map_err(caught);
            let _ = ctx.globals().remove("__player");
            result
        });
        self.runtime.run_gc();
        result
    }

    fn solve(&self, challenges: &Challenges) -> Result<Solutions> {
        let mut solutions = Solutions::default();
        if challenges.is_empty() {
            return Ok(solutions);
        }
        self.limit(SOLVE_LIMIT);
        self.context.with(|ctx| {
            let solver: Object = ctx.globals().get("__solver").map_err(js)?;
            let run = |kind: &str,
                       inputs: &HashSet<String>,
                       out: &mut HashMap<String, String>|
             -> Result<()> {
                if inputs.is_empty() {
                    return Ok(());
                }
                let function: Option<Function> = solver.get(kind).map_err(js)?;
                let function = function.ok_or_else(|| {
                    Error::Streams(format!(
                        "the challenge solver found no {kind} function in this player"
                    ))
                })?;
                for input in inputs {
                    let output: String = function
                        .call((input.as_str(),))
                        .catch(&ctx)
                        .map_err(caught)?;
                    out.insert(input.clone(), output);
                }
                Ok(())
            };
            run("n", &challenges.n, &mut solutions.n)?;
            run("sig", &challenges.sig, &mut solutions.sig)
        })?;
        Ok(solutions)
    }
}

/// The bundled solver, or a newer one in the config directory's `solver/`.
fn scripts() -> (String, String) {
    let dir = crate::paths::config_dir().join("solver");
    match (
        std::fs::read_to_string(dir.join("lib.min.js")),
        std::fs::read_to_string(dir.join("core.min.js")),
    ) {
        (Ok(lib), Ok(core)) => {
            tracing::info!("using the challenge solver in {}", dir.display());
            (lib, core)
        }
        _ => (LIB.to_owned(), CORE.to_owned()),
    }
}

fn js(e: rquickjs::Error) -> Error {
    Error::Streams(format!("challenge solver: {e}"))
}

fn caught(e: rquickjs::CaughtError<'_>) -> Error {
    Error::Streams(format!("challenge solver: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_loads_the_solver() {
        let engine = Engine::new().expect("QuickJS and the solver load");
        engine.context.with(|ctx| {
            let kind: String = ctx.eval("typeof jsc").unwrap();
            assert_eq!(kind, "function");
        });
    }

    #[test]
    fn preprocessed_files_stay_in_their_directory() {
        let dir = Path::new("/cache/solver");
        assert_eq!(preprocessed_file(dir, "5203c085"), dir.join("5203c085.js"));
        assert_eq!(preprocessed_file(dir, "../../etc/x"), dir.join("etcx.js"));
    }
}
