//! The image studio: pictures from words, or from pictures and words.
//!
//! Fork addition. A row under Search in the sidebar opens it in the main
//! column — beside the sidebar rather than over it the way Settings is, so
//! the tasks stay a click away. One field takes the prompt; pictures dropped
//! on the page, pasted into the field or picked from disk turn the request
//! into an edit. What it draws lands in the Pictures folder and in a gallery
//! above the field (`image_studio_view.rs`).
//!
//! The requests are `sub2api::images`: a task where the gateway has them —
//! polled, and picked up again when the page next opens — otherwise one
//! streamed call. Image generation is granted per group and the catalog does
//! not say which groups have it, so a job walks the candidate groups past a
//! refusal, and the group that draws is remembered in
//! `Credentials::image_groups`, where the agents' routing picks it up too.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sub2api::images::{
    self, ImageError, ImageErrorKind, ImageGroup, ImageOutput, ImageRoute, ImageSpec, TaskState,
};

use super::*;

/// Jobs drawing at once; the gateway limits image concurrency per user too.
const MAX_RUNNING: usize = 3;
/// A group that refused image generation is passed over for this long.
const DENIAL_TTL_SECONDS: i64 = 24 * 60 * 60;
/// Network failures tolerated in a row while polling a task.
const POLL_NETWORK_RETRIES: u32 = 5;

pub(super) struct ImageStudioState {
    pub(super) open: bool,
    pub(super) input: Option<Entity<TextInput>>,
    pub(super) references: Vec<StudioReference>,
    pub(super) store: StudioStore,
    pub(super) loaded: bool,
    load_started: bool,
    /// Jobs with a driver running now; a running job outside it is queued.
    pub(super) running: HashSet<Uuid>,
    /// The gateway said it has no asynchronous tasks; ask synchronously for
    /// the rest of this run.
    async_unavailable: bool,
    ticking: bool,
    pub(super) thumbs: RefCell<HashMap<PathBuf, Thumb>>,
    pub(super) scroll: ScrollHandle,
    pub(super) scrollbar: Rc<ScrollbarState>,
}

impl Default for ImageStudioState {
    fn default() -> Self {
        Self {
            open: false,
            input: None,
            references: Vec::new(),
            store: StudioStore::default(),
            loaded: false,
            load_started: false,
            running: HashSet::new(),
            async_unavailable: false,
            ticking: false,
            thumbs: RefCell::new(HashMap::new()),
            scroll: ScrollHandle::new(),
            scrollbar: ScrollbarState::new(),
        }
    }
}

/// A picture the next request starts from.
pub(super) struct StudioReference {
    pub(super) path: PathBuf,
    pub(super) image: Option<Arc<gpui::Image>>,
}

/// A gallery picture as loaded for display.
pub(super) enum Thumb {
    Loading,
    Ready(Arc<gpui::Image>),
    Missing,
}

/// What the studio keeps between runs: `~/.cheaprouter/image-studio/index.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct StudioStore {
    #[serde(default)]
    pub(super) prefs: StudioPrefs,
    /// Groups that refused a model, so the next job does not ask them first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) denials: Vec<Denial>,
    /// Newest first.
    #[serde(default)]
    pub(super) jobs: Vec<StudioJob>,
}

/// The last choices made in the controls row.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct StudioPrefs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) model: Option<String>,
    /// A group picked by hand; `None` lets each job find one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) group: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) quality: Option<String>,
    #[serde(default)]
    pub(super) count: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Denial {
    pub(super) model: String,
    pub(super) group_id: i64,
    pub(super) at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct StudioJob {
    pub(super) id: Uuid,
    pub(super) created_at: i64,
    pub(super) spec: ImageSpec,
    /// A group picked by hand for this job; otherwise the candidates walk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) forced_group: Option<i64>,
    /// The group being asked, or the one that drew.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) group_id: Option<i64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) group_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) estimate_usd: Option<f64>,
    /// File names start with this: `YYYYMMDD-HHMMSS-<id>`.
    pub(super) stem: String,
    /// The month folder the pictures go in.
    pub(super) folder: String,
    pub(super) status: JobStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(super) enum JobStatus {
    Running {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task_id: Option<String>,
        started_at: i64,
    },
    Done {
        outputs: Vec<PathBuf>,
        finished_at: i64,
    },
    Failed {
        error: ImageError,
        finished_at: i64,
    },
}

impl StudioJob {
    pub(super) fn is_running(&self) -> bool {
        matches!(self.status, JobStatus::Running { .. })
    }

    fn task_id(&self) -> Option<&str> {
        match &self.status {
            JobStatus::Running { task_id, .. } => task_id.as_deref(),
            _ => None,
        }
    }
}

impl StudioStore {
    fn dir() -> Option<PathBuf> {
        sub2api::brand::data_dir().map(|dir| dir.join("image-studio"))
    }

    fn path() -> Option<PathBuf> {
        Self::dir().map(|dir| dir.join("index.json"))
    }

    pub(super) fn references_dir() -> Option<PathBuf> {
        Self::dir().map(|dir| dir.join("refs"))
    }

    fn load_from(path: &Path) -> Self {
        let Ok(raw) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&raw).unwrap_or_else(|error| {
            eprintln!("warning: {} is unreadable: {error}", path.display());
            Self::default()
        })
    }

    /// Settle what a restart left behind. A job drawing synchronously, or
    /// still waiting for a slot, had nothing to come back to; a task past the
    /// gateway's retention is gone.
    pub(super) fn recover(&mut self, now: i64) {
        for job in &mut self.jobs {
            let JobStatus::Running {
                task_id,
                started_at,
            } = &job.status
            else {
                continue;
            };
            let error = match task_id {
                None => ImageError::new(ImageErrorKind::Interrupted, "interrupted"),
                Some(_) if now - started_at > images::TASK_TTL_SECONDS => {
                    ImageError::new(ImageErrorKind::TaskLost, "expired")
                }
                Some(_) => continue,
            };
            job.status = JobStatus::Failed {
                error,
                finished_at: now,
            };
        }
        self.denials
            .retain(|denial| now - denial.at < DENIAL_TTL_SECONDS);
    }

    pub(super) fn denied(&self, model: &str, group_id: i64, now: i64) -> bool {
        self.denials.iter().any(|denial| {
            denial.group_id == group_id
                && denial.model.eq_ignore_ascii_case(model)
                && now - denial.at < DENIAL_TTL_SECONDS
        })
    }

    fn deny(&mut self, model: &str, group_id: i64, now: i64) {
        self.denials.retain(|denial| {
            !(denial.group_id == group_id && denial.model.eq_ignore_ascii_case(model))
        });
        self.denials.push(Denial {
            model: model.to_owned(),
            group_id,
            at: now,
        });
    }

    pub(super) fn job(&self, id: Uuid) -> Option<&StudioJob> {
        self.jobs.iter().find(|job| job.id == id)
    }

    fn job_mut(&mut self, id: Uuid) -> Option<&mut StudioJob> {
        self.jobs.iter_mut().find(|job| job.id == id)
    }
}

/// Writes land in order: a save that finds a later one already written
/// drops itself.
static LAST_SAVE: std::sync::Mutex<u64> = std::sync::Mutex::new(0);
static SAVE_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn now_unix() -> i64 {
    Utc::now().timestamp()
}

/// The groups a new job for `model` walks: the one picked by hand alone, or
/// the candidates without the ones that refused lately — unless every one
/// did, when they are all asked again.
pub(super) fn job_candidates(
    candidates: Vec<ImageGroup>,
    forced: Option<i64>,
    store: &StudioStore,
    model: &str,
    now: i64,
) -> Vec<ImageGroup> {
    if let Some(forced) = forced {
        let group = candidates
            .into_iter()
            .find(|group| group.group_id == forced)
            .unwrap_or(ImageGroup {
                group_id: forced,
                name: String::new(),
                platform: String::new(),
                rate_multiplier: 0.0,
            });
        return vec![group];
    }
    let open: Vec<ImageGroup> = candidates
        .iter()
        .filter(|group| !store.denied(model, group.group_id, now))
        .cloned()
        .collect();
    if open.is_empty() { candidates } else { open }
}

/// A refusal that says "not here", after which the next group is asked.
fn try_next_group(error: &ImageError) -> bool {
    matches!(
        error.kind,
        ImageErrorKind::NoImagePermission
            | ImageErrorKind::ModelUnavailable
            | ImageErrorKind::WrongPlatform
    )
}

/// Everything a job's driver needs, taken from the app before it starts.
struct JobPlan {
    job_id: Uuid,
    spec: ImageSpec,
    origin: String,
    credentials: sub2api::Credentials,
    candidates: Vec<ImageGroup>,
    resume: Option<(i64, String)>,
    try_async: bool,
    output_dir: PathBuf,
    stem: String,
}

enum JobEnd {
    Done {
        outputs: Vec<PathBuf>,
        group_id: Option<i64>,
        credentials: sub2api::Credentials,
    },
    Failed(ImageError, sub2api::Credentials),
    Cancelled,
}

/// The key for `group_id`, minting one when the account has none yet.
fn group_key(credentials: &mut sub2api::Credentials, group_id: i64) -> anyhow::Result<String> {
    if let Some(key) = sub2api::key_for_group(credentials, group_id) {
        return Ok(key.to_owned());
    }
    sub2api::ensure_group_keys(credentials, &BTreeSet::from([group_id]))?;
    sub2api::key_for_group(credentials, group_id)
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("no key could be made for group {group_id}"))
}

async fn job_exists(this: &WeakEntity<Waku>, job_id: Uuid, cx: &mut gpui::AsyncApp) -> bool {
    this.update(cx, |this, _| this.image_studio.store.job(job_id).is_some())
        .unwrap_or(false)
}

/// Poll `task_id` until it ends. `Ok(None)` means the job was removed
/// meanwhile.
async fn poll_until_done(
    this: &WeakEntity<Waku>,
    job_id: Uuid,
    route: &ImageRoute,
    task_id: &str,
    cx: &mut gpui::AsyncApp,
) -> Result<Option<Vec<ImageOutput>>, ImageError> {
    let deadline = Instant::now() + Duration::from_secs(images::TASK_TIMEOUT_SECONDS as u64 + 120);
    let mut failures = 0;
    loop {
        cx.background_executor()
            .timer(Duration::from_secs(images::POLL_INTERVAL_SECONDS))
            .await;
        if !job_exists(this, job_id, cx).await {
            return Ok(None);
        }
        let (route, task) = (route.clone(), task_id.to_owned());
        let state = cx
            .background_executor()
            .spawn(async move { images::poll_task(&route, &task) })
            .await;
        match state {
            Ok(TaskState::Processing) => failures = 0,
            Ok(TaskState::Completed(outputs)) => return Ok(Some(outputs)),
            Ok(TaskState::Failed(error)) => return Err(error),
            Err(error)
                if error.kind == ImageErrorKind::Network && failures < POLL_NETWORK_RETRIES =>
            {
                failures += 1;
            }
            Err(error) => return Err(error),
        }
        if Instant::now() > deadline {
            return Err(ImageError::new(
                ImageErrorKind::Timeout,
                "the image task did not finish in time",
            ));
        }
    }
}

async fn save_outputs(
    outputs: Vec<ImageOutput>,
    dir: PathBuf,
    stem: String,
    cx: &mut gpui::AsyncApp,
) -> Result<Vec<PathBuf>, ImageError> {
    cx.background_executor()
        .spawn(async move {
            outputs
                .iter()
                .enumerate()
                .map(|(index, output)| {
                    images::save_output(output, &dir.join(format!("{stem}-{}", index + 1)))
                })
                .collect::<anyhow::Result<Vec<_>>>()
        })
        .await
        .map_err(|error| ImageError::new(ImageErrorKind::Other, format!("{error:#}")))
}

async fn run_image_job(this: WeakEntity<Waku>, plan: JobPlan, cx: &mut gpui::AsyncApp) -> JobEnd {
    let JobPlan {
        job_id,
        spec,
        origin,
        mut credentials,
        candidates,
        resume,
        mut try_async,
        output_dir,
        stem,
    } = plan;

    // A task submitted before the app closed: only the key that submitted
    // it may read it.
    if let Some((group_id, task_id)) = resume {
        let Some(key) = sub2api::key_for_group(&credentials, group_id).map(str::to_owned) else {
            return JobEnd::Failed(
                ImageError::new(ImageErrorKind::TaskLost, "the group's key is gone"),
                credentials,
            );
        };
        let route = ImageRoute {
            origin,
            api_key: key,
        };
        return match poll_until_done(&this, job_id, &route, &task_id, cx).await {
            Ok(None) => JobEnd::Cancelled,
            Ok(Some(outputs)) => match save_outputs(outputs, output_dir, stem, cx).await {
                Ok(outputs) => JobEnd::Done {
                    outputs,
                    group_id: Some(group_id),
                    credentials,
                },
                Err(error) => JobEnd::Failed(error, credentials),
            },
            Err(error) => JobEnd::Failed(error, credentials),
        };
    }

    let mut last_error = None;
    for group in candidates {
        if !job_exists(&this, job_id, cx).await {
            return JobEnd::Cancelled;
        }
        let group_id = group.group_id;
        let fetched = cx
            .background_executor()
            .spawn({
                let mut credentials = credentials.clone();
                async move {
                    let key = group_key(&mut credentials, group_id)?;
                    anyhow::Ok((credentials, key))
                }
            })
            .await;
        let key = match fetched {
            Ok((renewed, key)) => {
                credentials = renewed;
                key
            }
            Err(error) => {
                if sub2api::session_ended(&error) {
                    return JobEnd::Failed(
                        ImageError::new(ImageErrorKind::Unauthorized, format!("{error:#}")),
                        credentials,
                    );
                }
                last_error = Some(ImageError::network(error));
                continue;
            }
        };
        let _ = this.update(cx, |this, cx| {
            if let Some(job) = this.image_studio.store.job_mut(job_id) {
                job.group_id = Some(group_id);
                job.group_name = group.name.clone();
            }
            cx.notify();
        });
        let route = ImageRoute {
            origin: origin.clone(),
            api_key: key,
        };

        let drawn: Result<Option<Vec<ImageOutput>>, ImageError> = 'attempt: {
            if try_async {
                let submitted = cx
                    .background_executor()
                    .spawn({
                        let (route, spec) = (route.clone(), spec.clone());
                        async move { images::submit_async(&route, &spec) }
                    })
                    .await;
                match submitted {
                    Ok(task_id) => {
                        let _ = this.update(cx, |this, cx| {
                            if let Some(job) = this.image_studio.store.job_mut(job_id)
                                && let JobStatus::Running { task_id: slot, .. } = &mut job.status
                            {
                                *slot = Some(task_id.clone());
                            }
                            this.save_image_studio(cx);
                        });
                        break 'attempt poll_until_done(&this, job_id, &route, &task_id, cx).await;
                    }
                    Err(error) if error.kind == ImageErrorKind::AsyncUnavailable => {
                        try_async = false;
                        let _ =
                            this.update(cx, |this, _| this.image_studio.async_unavailable = true);
                    }
                    // A composite group has no tasks but may draw directly.
                    Err(error) if error.kind == ImageErrorKind::WrongPlatform => {}
                    Err(error) => break 'attempt Err(error),
                }
            }
            cx.background_executor()
                .spawn({
                    let (route, spec) = (route.clone(), spec.clone());
                    async move { images::generate_sync(&route, &spec) }
                })
                .await
                .map(Some)
        };

        match drawn {
            Ok(None) => return JobEnd::Cancelled,
            Ok(Some(outputs)) => {
                return match save_outputs(outputs, output_dir, stem, cx).await {
                    Ok(outputs) => JobEnd::Done {
                        outputs,
                        group_id: Some(group_id),
                        credentials,
                    },
                    Err(error) => JobEnd::Failed(error, credentials),
                };
            }
            Err(error) if try_next_group(&error) => {
                let model = spec.model().to_owned();
                let _ = this.update(cx, |this, cx| {
                    this.record_image_denial(&model, group_id, cx)
                });
                last_error = Some(error);
            }
            Err(error) => return JobEnd::Failed(error, credentials),
        }
    }
    JobEnd::Failed(
        last_error.unwrap_or_else(|| {
            ImageError::new(
                ImageErrorKind::NoImagePermission,
                "no group can draw this model",
            )
        }),
        credentials,
    )
}

impl Waku {
    // ── Opening ──────────────────────────────────────────────────────────

    pub(super) fn open_image_studio(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_page = None;
        self.image_studio.open = true;
        self.ensure_image_studio_loaded(cx);
        self.load_model_plaza_if_needed(false, cx);
        if self.cloud_account.credentials.is_some() {
            self.refresh_cloud_account(cx);
        }
        let input = self.ensure_image_studio_input(window, cx);
        window.focus(&input.read(cx).focus(), cx);
        cx.notify();
    }

    fn ensure_image_studio_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextInput> {
        if let Some(input) = self.image_studio.input.clone() {
            return input;
        }
        let input = cx.new(|cx| {
            TextInput::new(window, cx)
                .multi_line()
                .auto_height()
                .submit_on_enter()
                .media_paste()
                .placeholder(tr!("image_studio.placeholder"))
        });
        cx.subscribe(
            &input,
            |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Submit(_) => this.submit_image_studio(cx),
                InputEvent::Edited => cx.notify(),
                _ => {}
            },
        )
        .detach();
        cx.subscribe(
            &input,
            |this: &mut Self, _, event: &crate::input::MediaPaste, cx| {
                this.add_image_studio_pasted(event.0.clone(), cx);
            },
        )
        .detach();
        self.image_studio.input = Some(input.clone());
        input
    }

    fn ensure_image_studio_loaded(&mut self, cx: &mut Context<Self>) {
        if self.image_studio.load_started {
            return;
        }
        self.image_studio.load_started = true;
        cx.spawn(async move |this, cx| {
            let store = cx
                .background_executor()
                .spawn(async move {
                    let mut store = StudioStore::path()
                        .map(|path| StudioStore::load_from(&path))
                        .unwrap_or_default();
                    store.recover(now_unix());
                    store
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                // Jobs started before the file was read stay in front.
                let started = std::mem::take(&mut this.image_studio.store.jobs);
                this.image_studio.store = StudioStore {
                    jobs: started.into_iter().chain(store.jobs).collect(),
                    ..store
                };
                this.image_studio.loaded = true;
                this.pump_image_jobs(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn save_image_studio(&self, cx: &mut Context<Self>) {
        let Some(path) = StudioStore::path() else {
            return;
        };
        if !self.image_studio.loaded {
            // Saving before the file is read would overwrite it.
            return;
        }
        let store = self.image_studio.store.clone();
        let sequence = SAVE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        cx.background_executor()
            .spawn(async move {
                let mut last = LAST_SAVE
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if *last > sequence {
                    return;
                }
                *last = sequence;
                let result = serde_json::to_vec_pretty(&store)
                    .map_err(anyhow::Error::from)
                    .and_then(|bytes| sub2api::global_config::atomic_write_private(&path, &bytes));
                if let Err(error) = result {
                    eprintln!("warning: could not save the image studio: {error:#}");
                }
            })
            .detach();
    }

    // ── Choices ──────────────────────────────────────────────────────────

    /// The image models the catalog lists, or the gateway's default when it
    /// lists none (an account whose groups map no models lists nothing).
    pub(super) fn image_studio_models(&self) -> Vec<String> {
        let mut models: Vec<String> = self
            .model_plaza
            .items
            .iter()
            .filter(|item| images::is_image_model(&item.model))
            .map(|item| item.model.trim().to_owned())
            .collect();
        models.sort();
        models.dedup();
        if models.is_empty() {
            models.push(images::DEFAULT_MODEL.to_owned());
        }
        models
    }

    pub(super) fn image_studio_model(&self) -> String {
        let models = self.image_studio_models();
        let prefs = &self.image_studio.store.prefs;
        prefs
            .model
            .as_ref()
            .filter(|model| models.contains(model))
            .cloned()
            .or_else(|| {
                models
                    .iter()
                    .find(|model| *model == images::DEFAULT_MODEL)
                    .cloned()
            })
            .unwrap_or_else(|| models[0].clone())
    }

    pub(super) fn image_studio_candidates(&self, model: &str) -> Vec<ImageGroup> {
        let Some(credentials) = self.cloud_account.credentials.as_ref() else {
            return Vec::new();
        };
        images::image_route_candidates(credentials, &self.model_plaza.items, model)
    }

    pub(super) fn image_studio_size(&self, model: &str) -> String {
        let options = images::size_options(model);
        self.image_studio
            .store
            .prefs
            .size
            .as_deref()
            .filter(|size| options.contains(size))
            .unwrap_or(options[0])
            .to_owned()
    }

    pub(super) fn image_studio_quality(&self, model: &str) -> Option<String> {
        if !images::supports_quality(model) {
            return None;
        }
        self.image_studio
            .store
            .prefs
            .quality
            .clone()
            .filter(|quality| images::QUALITY_OPTIONS.contains(&quality.as_str()))
    }

    pub(super) fn image_studio_count(&self) -> u8 {
        self.image_studio
            .store
            .prefs
            .count
            .clamp(1, images::MAX_COUNT)
    }

    /// The group a new job would ask first, for pricing.
    fn image_studio_price_item(&self, model: &str) -> Option<&sub2api::client::ModelCatalogItem> {
        let for_group = |group: i64| {
            self.model_plaza.items.iter().find(|item| {
                item.best_group.id == group && item.model.trim().eq_ignore_ascii_case(model)
            })
        };
        if let Some(group) = self.image_studio.store.prefs.group {
            return for_group(group);
        }
        self.image_studio_candidates(model)
            .iter()
            .find_map(|group| for_group(group.group_id))
            .or_else(|| {
                self.model_plaza
                    .items
                    .iter()
                    .find(|item| item.model.trim().eq_ignore_ascii_case(model))
            })
    }

    pub(super) fn image_studio_estimate(&self) -> Option<f64> {
        let model = self.image_studio_model();
        let size = self.image_studio_size(&model);
        let item = self.image_studio_price_item(&model)?;
        images::estimate_usd(item, Some(&size), self.image_studio_count())
    }

    /// One picture's price at `size`, for the size menu.
    pub(super) fn image_studio_unit_price(&self, model: &str, size: &str) -> Option<f64> {
        let item = self.image_studio_price_item(model)?;
        images::estimate_usd(item, Some(size), 1)
    }

    pub(super) fn set_image_studio_model(&mut self, model: String, cx: &mut Context<Self>) {
        let prefs = &mut self.image_studio.store.prefs;
        if prefs.model.as_deref() != Some(model.as_str()) {
            // A group picked for one model rarely serves the next.
            prefs.group = None;
        }
        prefs.model = Some(model);
        self.trim_image_studio_references(cx);
        self.save_image_studio(cx);
        cx.notify();
    }

    pub(super) fn set_image_studio_group(&mut self, group: Option<i64>, cx: &mut Context<Self>) {
        self.image_studio.store.prefs.group = group;
        self.save_image_studio(cx);
        cx.notify();
    }

    pub(super) fn set_image_studio_size(&mut self, size: String, cx: &mut Context<Self>) {
        self.image_studio.store.prefs.size = Some(size);
        self.save_image_studio(cx);
        cx.notify();
    }

    pub(super) fn set_image_studio_quality(
        &mut self,
        quality: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.image_studio.store.prefs.quality = quality;
        self.save_image_studio(cx);
        cx.notify();
    }

    pub(super) fn set_image_studio_count(&mut self, count: u8, cx: &mut Context<Self>) {
        self.image_studio.store.prefs.count = count.clamp(1, images::MAX_COUNT);
        self.save_image_studio(cx);
        cx.notify();
    }

    // ── References ───────────────────────────────────────────────────────

    fn trim_image_studio_references(&mut self, cx: &mut Context<Self>) {
        let limit = images::max_references(&self.image_studio_model());
        if self.image_studio.references.len() > limit {
            self.image_studio.references.truncate(limit);
            self.show_toast(tr!("image_studio.too_many_references", count = limit));
            cx.notify();
        }
    }

    /// Take pictures from disk: checked, then copied beside the studio's
    /// index so a retry still finds them after the original moves.
    pub(super) fn add_image_studio_references(
        &mut self,
        paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let Some(dir) = StudioStore::references_dir() else {
            return;
        };
        let room = images::max_references(&self.image_studio_model())
            .saturating_sub(self.image_studio.references.len());
        cx.spawn(async move |this, cx| {
            let (added, problems) = cx
                .background_executor()
                .spawn(async move {
                    let mut added = Vec::new();
                    let mut problems = Vec::new();
                    for path in paths {
                        if added.len() >= room {
                            problems.push(ReferenceProblem::TooMany);
                            break;
                        }
                        match copy_reference(&path, &dir) {
                            Ok(reference) => added.push(reference),
                            Err(problem) => problems.push(problem),
                        }
                    }
                    (added, problems)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.image_studio
                    .references
                    .extend(added.into_iter().map(|(path, image)| StudioReference {
                        path,
                        image: Some(image),
                    }));
                if let Some(problem) = problems.first() {
                    let limit = images::max_references(&this.image_studio_model());
                    this.show_toast(problem.message(limit));
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn add_image_studio_pasted(&mut self, entries: Vec<ClipboardEntry>, cx: &mut Context<Self>) {
        let mut paths = Vec::new();
        let mut pictures = Vec::new();
        for entry in entries {
            match entry {
                ClipboardEntry::Image(image) if !image.bytes.is_empty() => pictures.push(image),
                ClipboardEntry::ExternalPaths(external) => {
                    paths.extend(external.paths().iter().cloned());
                }
                _ => {}
            }
        }
        if !paths.is_empty() {
            self.add_image_studio_references(paths, cx);
        }
        if pictures.is_empty() {
            return;
        }
        let Some(dir) = StudioStore::references_dir() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let written = cx
                .background_executor()
                .spawn(async move {
                    pictures
                        .into_iter()
                        .map(|image| {
                            let extension = match image.format {
                                gpui::ImageFormat::Png => "png",
                                gpui::ImageFormat::Jpeg => "jpg",
                                gpui::ImageFormat::Webp => "webp",
                                _ => return Err(ReferenceProblem::Format),
                            };
                            if image.bytes.len() as u64 > images::MAX_REFERENCE_BYTES {
                                return Err(ReferenceProblem::TooLarge);
                            }
                            let path = dir.join(format!("{}.{extension}", Uuid::new_v4()));
                            std::fs::create_dir_all(&dir)
                                .map_err(|_| ReferenceProblem::Unreadable)?;
                            std::fs::write(&path, &image.bytes)
                                .map_err(|_| ReferenceProblem::Unreadable)?;
                            Ok(StudioReference {
                                path,
                                image: Some(Arc::new(image)),
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let limit = images::max_references(&this.image_studio_model());
                for result in written {
                    match result {
                        Ok(reference) if this.image_studio.references.len() < limit => {
                            this.image_studio.references.push(reference);
                        }
                        Ok(_) => this.show_toast(ReferenceProblem::TooMany.message(limit)),
                        Err(problem) => this.show_toast(problem.message(limit)),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn pick_image_studio_references(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(tr!("image_studio.add_images").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await {
                let _ = this.update(cx, |this, cx| this.add_image_studio_references(paths, cx));
            }
        })
        .detach();
    }

    pub(super) fn remove_image_studio_reference(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.image_studio.references.len() {
            self.image_studio.references.remove(index);
            cx.notify();
        }
    }

    /// A drawn picture as the start of the next edit.
    pub(super) fn use_image_as_reference(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.add_image_studio_references(vec![path], cx);
    }

    // ── Jobs ─────────────────────────────────────────────────────────────

    pub(super) fn image_studio_can_submit(&self, cx: &App) -> bool {
        self.cloud_account.credentials.is_some()
            && self
                .image_studio
                .input
                .as_ref()
                .is_some_and(|input| !input.read(cx).content().trim().is_empty())
    }

    pub(super) fn submit_image_studio(&mut self, cx: &mut Context<Self>) {
        if self.cloud_account.credentials.is_none() {
            self.show_toast(tr!("image_studio.sign_in_first"));
            cx.notify();
            return;
        }
        let Some(input) = self.image_studio.input.clone() else {
            return;
        };
        let prompt = input.read(cx).content().trim().to_owned();
        if prompt.is_empty() {
            return;
        }
        let model = self.image_studio_model();
        let size = self.image_studio_size(&model);
        let spec = ImageSpec {
            prompt,
            size: Some(size),
            quality: self
                .image_studio_quality(&model)
                .filter(|quality| quality != "auto"),
            count: self.image_studio_count(),
            references: self
                .image_studio
                .references
                .iter()
                .map(|reference| reference.path.clone())
                .collect(),
            model,
            ..ImageSpec::default()
        };
        let estimate_usd = self.image_studio_estimate();
        let forced_group = self.image_studio.store.prefs.group;
        self.push_image_job(spec, forced_group, estimate_usd, cx);
        input.update(cx, |input, cx| input.clear(cx));
        self.image_studio.references.clear();
        self.image_studio.scroll.set_offset(gpui::Point::default());
        cx.notify();
    }

    fn push_image_job(
        &mut self,
        spec: ImageSpec,
        forced_group: Option<i64>,
        estimate_usd: Option<f64>,
        cx: &mut Context<Self>,
    ) {
        let id = Uuid::new_v4();
        let now = Local::now();
        let short: String = id.simple().to_string().chars().take(6).collect();
        self.image_studio.store.jobs.insert(
            0,
            StudioJob {
                id,
                created_at: now.timestamp(),
                spec,
                forced_group,
                group_id: None,
                group_name: String::new(),
                estimate_usd,
                stem: format!("{}-{short}", now.format("%Y%m%d-%H%M%S")),
                folder: now.format("%Y-%m").to_string(),
                status: JobStatus::Running {
                    task_id: None,
                    started_at: now.timestamp(),
                },
            },
        );
        self.save_image_studio(cx);
        self.pump_image_jobs(cx);
    }

    /// Start drivers for waiting jobs, oldest first, up to the limit.
    fn pump_image_jobs(&mut self, cx: &mut Context<Self>) {
        if !self.image_studio.loaded {
            return;
        }
        let waiting: Vec<Uuid> = self
            .image_studio
            .store
            .jobs
            .iter()
            .rev()
            .filter(|job| job.is_running() && !self.image_studio.running.contains(&job.id))
            .map(|job| job.id)
            .collect();
        for id in waiting {
            if self.image_studio.running.len() >= MAX_RUNNING {
                break;
            }
            self.drive_image_job(id, cx);
        }
        self.ensure_image_studio_ticker(cx);
    }

    fn drive_image_job(&mut self, job_id: Uuid, cx: &mut Context<Self>) {
        let Some(job) = self.image_studio.store.job(job_id).cloned() else {
            return;
        };
        let Some(credentials) = self.cloud_account.credentials.clone() else {
            self.fail_image_job(
                job_id,
                ImageError::new(ImageErrorKind::Unauthorized, "signed out"),
                cx,
            );
            return;
        };
        let Some(output_dir) = images::pictures_dir().map(|dir| dir.join(&job.folder)) else {
            self.fail_image_job(
                job_id,
                ImageError::new(ImageErrorKind::Other, "no folder to save pictures in"),
                cx,
            );
            return;
        };
        let origin = self
            .cloud_account
            .gateway_origin
            .origin()
            .unwrap_or_else(|| credentials.endpoint.clone());
        let model = job.spec.model().to_owned();
        let resume = job
            .task_id()
            .and_then(|task| Some((job.group_id?, task.to_owned())));
        let candidates = job_candidates(
            self.image_studio_candidates(&model),
            job.forced_group,
            &self.image_studio.store,
            &model,
            now_unix(),
        );
        let plan = JobPlan {
            job_id,
            spec: job.spec.clone(),
            origin,
            credentials,
            candidates,
            resume,
            try_async: !self.image_studio.async_unavailable,
            output_dir,
            stem: job.stem.clone(),
        };
        self.image_studio.running.insert(job_id);
        if let Some(job) = self.image_studio.store.job_mut(job_id)
            && let JobStatus::Running {
                started_at,
                task_id,
            } = &mut job.status
            && task_id.is_none()
        {
            // A queued job starts its clock when it starts drawing.
            *started_at = now_unix();
        }
        cx.spawn(async move |this, cx| {
            let end = run_image_job(this.clone(), plan, cx).await;
            let _ = this.update(cx, |this, cx| this.finish_image_job(job_id, end, cx));
        })
        .detach();
    }

    fn finish_image_job(&mut self, job_id: Uuid, end: JobEnd, cx: &mut Context<Self>) {
        self.image_studio.running.remove(&job_id);
        let now = now_unix();
        match end {
            JobEnd::Done {
                outputs,
                group_id,
                credentials,
            } => {
                let model = self
                    .image_studio
                    .store
                    .job(job_id)
                    .map(|job| job.spec.model().to_owned());
                if let Some(job) = self.image_studio.store.job_mut(job_id) {
                    job.status = JobStatus::Done {
                        outputs,
                        finished_at: now,
                    };
                }
                self.adopt_image_credentials(credentials, model.as_deref().zip(group_id));
                // What was drawn is paid for; show the balance it left.
                self.refresh_cloud_account(cx);
            }
            JobEnd::Failed(error, credentials) => {
                self.adopt_image_credentials(credentials, None);
                if let Some(job) = self.image_studio.store.job_mut(job_id) {
                    job.status = JobStatus::Failed {
                        error,
                        finished_at: now,
                    };
                }
            }
            JobEnd::Cancelled => {}
        }
        self.save_image_studio(cx);
        self.pump_image_jobs(cx);
        cx.notify();
    }

    fn fail_image_job(&mut self, job_id: Uuid, error: ImageError, cx: &mut Context<Self>) {
        if let Some(job) = self.image_studio.store.job_mut(job_id) {
            job.status = JobStatus::Failed {
                error,
                finished_at: now_unix(),
            };
        }
        self.save_image_studio(cx);
        cx.notify();
    }

    /// Take what a job's copy of the session learned: renewed tokens, keys
    /// minted for groups, and — after a picture — the group that drew it,
    /// which also becomes the model's route so the agents' image calls go
    /// there.
    fn adopt_image_credentials(
        &mut self,
        renewed: sub2api::Credentials,
        drew: Option<(&str, i64)>,
    ) {
        let keys = renewed.group_keys.clone();
        self.adopt_cloud_tokens(renewed);
        let Some(credentials) = self.cloud_account.credentials.as_mut() else {
            return;
        };
        let mut changed = false;
        for (group, key) in keys {
            if let std::collections::btree_map::Entry::Vacant(entry) =
                credentials.group_keys.entry(group)
            {
                entry.insert(key);
                changed = true;
            }
        }
        let mut routes_changed = false;
        if let Some((model, group)) = drew {
            if credentials.image_groups.get(model) != Some(&group) {
                credentials.image_groups.insert(model.to_owned(), group);
                changed = true;
            }
            if credentials.model_routes.get(model) != Some(&group) {
                credentials.model_routes.insert(model.to_owned(), group);
                changed = true;
                routes_changed = true;
            }
        }
        if !changed {
            return;
        }
        if let Err(error) = credentials.save() {
            self.show_toast(format!("{error:#}"));
        }
        if routes_changed {
            self.apply_cloud_routing();
        }
    }

    fn record_image_denial(&mut self, model: &str, group_id: i64, cx: &mut Context<Self>) {
        self.image_studio.store.deny(model, group_id, now_unix());
        // A group that stopped drawing is no longer the one to ask first.
        if let Some(credentials) = self.cloud_account.credentials.as_mut()
            && credentials.image_groups.get(model) == Some(&group_id)
        {
            credentials.image_groups.remove(model);
            if let Err(error) = credentials.save() {
                self.show_toast(format!("{error:#}"));
            }
        }
        self.save_image_studio(cx);
    }

    /// Redraw the running cards' clocks once a second while any job runs
    /// and the page is open.
    fn ensure_image_studio_ticker(&mut self, cx: &mut Context<Self>) {
        if self.image_studio.ticking || self.image_studio.running.is_empty() {
            return;
        }
        self.image_studio.ticking = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let keep = this
                    .update(cx, |this, cx| {
                        let keep = !this.image_studio.running.is_empty();
                        if !keep {
                            this.image_studio.ticking = false;
                        } else if this.image_studio.open && this.settings_page.is_none() {
                            cx.notify();
                        }
                        keep
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn retry_image_job(&mut self, job_id: Uuid, cx: &mut Context<Self>) {
        if let Some(job) = self.image_studio.store.job_mut(job_id) {
            job.group_id = None;
            job.group_name.clear();
            job.status = JobStatus::Running {
                task_id: None,
                started_at: now_unix(),
            };
        }
        self.save_image_studio(cx);
        self.pump_image_jobs(cx);
        cx.notify();
    }

    /// The same request again, as a new job.
    pub(super) fn rerun_image_job(&mut self, job_id: Uuid, cx: &mut Context<Self>) {
        let Some(job) = self.image_studio.store.job(job_id).cloned() else {
            return;
        };
        self.push_image_job(job.spec, job.forced_group, job.estimate_usd, cx);
        self.image_studio.scroll.set_offset(gpui::Point::default());
        cx.notify();
    }

    /// Take a job off the gallery. The pictures stay on disk; a running job
    /// stops being waited for (what the gateway already drew is billed).
    pub(super) fn remove_image_job(&mut self, job_id: Uuid, cx: &mut Context<Self>) {
        self.image_studio.store.jobs.retain(|job| job.id != job_id);
        self.image_studio.running.remove(&job_id);
        self.save_image_studio(cx);
        self.pump_image_jobs(cx);
        cx.notify();
    }

    /// The prompt and settings of a job, back in the field to adjust.
    pub(super) fn edit_image_job(
        &mut self,
        job_id: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(job) = self.image_studio.store.job(job_id).cloned() else {
            return;
        };
        let prefs = &mut self.image_studio.store.prefs;
        prefs.model = Some(job.spec.model().to_owned());
        prefs.size = job.spec.size.clone();
        prefs.quality = job.spec.quality.clone();
        prefs.count = job.spec.count;
        prefs.group = job.forced_group;
        let input = self.ensure_image_studio_input(window, cx);
        input.update(cx, |input, cx| {
            input.set_content(job.spec.prompt.clone(), cx)
        });
        window.focus(&input.read(cx).focus(), cx);
        let existing: Vec<PathBuf> = job
            .spec
            .references
            .into_iter()
            .filter(|path| path.exists())
            .collect();
        self.image_studio.references.clear();
        if !existing.is_empty() {
            self.add_image_studio_references(existing, cx);
        }
        self.save_image_studio(cx);
        cx.notify();
    }

    pub(super) fn copy_image_prompt(&mut self, job_id: Uuid, cx: &mut Context<Self>) {
        if let Some(job) = self.image_studio.store.job(job_id) {
            cx.write_to_clipboard(ClipboardItem::new_string(job.spec.prompt.clone()));
            self.show_toast(tr!("image_studio.prompt_copied"));
            cx.notify();
        }
    }

    pub(super) fn open_image_folder(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = images::pictures_dir() else {
            return;
        };
        if let Err(error) = std::fs::create_dir_all(&dir) {
            self.show_toast(format!("{error:#}"));
            cx.notify();
            return;
        }
        cx.open_with_system(&dir);
    }

    /// Hand a drawn picture to a task: staged as an attachment in the
    /// current task's composer, or a new task's when none is open.
    pub(super) fn send_image_to_chat(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.image_studio.open = false;
        if self.selected_session().is_none() {
            self.new_session_action(&NewSession, window, cx);
        }
        self.stage_attachment_paths(&[path], cx);
        let focus = self.composer_focus(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    pub(super) fn preview_studio_image(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let image = match self.image_studio.thumbs.borrow().get(&path) {
            Some(Thumb::Ready(image)) => image.clone(),
            _ => return,
        };
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.open_image_preview(image, SharedString::from(name), window, cx);
    }

    /// A drawn picture for a card: from memory, or read in the background
    /// with a redraw when it arrives.
    pub(super) fn studio_thumb(
        &self,
        path: &Path,
        cx: &mut Context<Self>,
    ) -> Option<Arc<gpui::Image>> {
        if let Some(thumb) = self.image_studio.thumbs.borrow().get(path) {
            return match thumb {
                Thumb::Ready(image) => Some(image.clone()),
                Thumb::Loading | Thumb::Missing => None,
            };
        }
        self.image_studio
            .thumbs
            .borrow_mut()
            .insert(path.to_owned(), Thumb::Loading);
        let path = path.to_owned();
        cx.spawn(async move |this, cx| {
            let read = cx
                .background_executor()
                .spawn({
                    let path = path.clone();
                    async move {
                        let format =
                            super::image_preview::image_format_for_name(&path.to_string_lossy())?;
                        let bytes = std::fs::read(&path).ok()?;
                        Some(Arc::new(gpui::Image::from_bytes(format, bytes)))
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let thumb = read.map_or(Thumb::Missing, Thumb::Ready);
                this.image_studio.thumbs.borrow_mut().insert(path, thumb);
                cx.notify();
            });
        })
        .detach();
        None
    }
}

/// Why a picture was not taken as a reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReferenceProblem {
    Format,
    TooLarge,
    TooMany,
    Unreadable,
}

impl ReferenceProblem {
    fn message(self, limit: usize) -> String {
        match self {
            Self::Format => tr!("image_studio.reference_format"),
            Self::TooLarge => tr!("image_studio.reference_too_large"),
            Self::TooMany => tr!("image_studio.too_many_references", count = limit),
            Self::Unreadable => tr!("image_studio.reference_unreadable"),
        }
    }
}

/// Check one picture and copy it into `dir` under a fresh name.
fn copy_reference(
    path: &Path,
    dir: &Path,
) -> Result<(PathBuf, Arc<gpui::Image>), ReferenceProblem> {
    let mime = images::reference_mime(path).ok_or(ReferenceProblem::Format)?;
    let size = std::fs::metadata(path)
        .map_err(|_| ReferenceProblem::Unreadable)?
        .len();
    if size > images::MAX_REFERENCE_BYTES {
        return Err(ReferenceProblem::TooLarge);
    }
    let bytes = std::fs::read(path).map_err(|_| ReferenceProblem::Unreadable)?;
    let (format, extension) = match mime {
        "image/jpeg" => (gpui::ImageFormat::Jpeg, "jpg"),
        "image/webp" => (gpui::ImageFormat::Webp, "webp"),
        _ => (gpui::ImageFormat::Png, "png"),
    };
    std::fs::create_dir_all(dir).map_err(|_| ReferenceProblem::Unreadable)?;
    let copy = dir.join(format!("{}.{extension}", Uuid::new_v4()));
    std::fs::write(&copy, &bytes).map_err(|_| ReferenceProblem::Unreadable)?;
    Ok((copy, Arc::new(gpui::Image::from_bytes(format, bytes))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(id: i64) -> ImageGroup {
        ImageGroup {
            group_id: id,
            name: format!("g{id}"),
            platform: "openai".into(),
            rate_multiplier: 1.0,
        }
    }

    fn job(status: JobStatus) -> StudioJob {
        StudioJob {
            id: Uuid::new_v4(),
            created_at: 0,
            spec: ImageSpec::default(),
            forced_group: None,
            group_id: Some(3),
            group_name: String::new(),
            estimate_usd: None,
            stem: "s".into(),
            folder: "2026-09".into(),
            status,
        }
    }

    #[test]
    fn a_restart_keeps_tasks_and_fails_what_cannot_come_back() {
        let now = 100_000;
        let mut store = StudioStore {
            jobs: vec![
                job(JobStatus::Running {
                    task_id: Some("imgtask_1".into()),
                    started_at: now - 60,
                }),
                job(JobStatus::Running {
                    task_id: None,
                    started_at: now - 60,
                }),
                job(JobStatus::Running {
                    task_id: Some("imgtask_2".into()),
                    started_at: now - images::TASK_TTL_SECONDS - 1,
                }),
            ],
            denials: vec![
                Denial {
                    model: "gpt-image-2".into(),
                    group_id: 1,
                    at: now - 10,
                },
                Denial {
                    model: "gpt-image-2".into(),
                    group_id: 2,
                    at: now - DENIAL_TTL_SECONDS - 1,
                },
            ],
            ..StudioStore::default()
        };
        store.recover(now);
        assert!(store.jobs[0].is_running());
        let kind = |job: &StudioJob| match &job.status {
            JobStatus::Failed { error, .. } => Some(error.kind),
            _ => None,
        };
        assert_eq!(kind(&store.jobs[1]), Some(ImageErrorKind::Interrupted));
        assert_eq!(kind(&store.jobs[2]), Some(ImageErrorKind::TaskLost));
        assert_eq!(store.denials.len(), 1);
        assert!(store.denied("GPT-IMAGE-2", 1, now));
        assert!(!store.denied("gpt-image-2", 2, now));
    }

    #[test]
    fn candidates_pass_over_refusals_unless_all_refused_or_one_was_picked() {
        let now = 1_000;
        let mut store = StudioStore::default();
        store.deny("gpt-image-2", 1, now);
        let ids = |groups: Vec<ImageGroup>| groups.iter().map(|g| g.group_id).collect::<Vec<_>>();

        let walk = job_candidates(vec![group(1), group(2)], None, &store, "gpt-image-2", now);
        assert_eq!(ids(walk), vec![2]);

        let all_refused = job_candidates(vec![group(1)], None, &store, "gpt-image-2", now);
        assert_eq!(ids(all_refused), vec![1]);

        let picked = job_candidates(vec![group(2)], Some(1), &store, "gpt-image-2", now);
        assert_eq!(ids(picked), vec![1]);

        // Denying again replaces the old entry rather than stacking.
        store.deny("gpt-image-2", 1, now + 5);
        assert_eq!(store.denials.len(), 1);
    }

    #[test]
    fn the_store_round_trips() {
        let store = StudioStore {
            prefs: StudioPrefs {
                model: Some("gpt-image-2".into()),
                group: Some(7),
                size: Some("1536x1024".into()),
                quality: None,
                count: 2,
            },
            jobs: vec![
                job(JobStatus::Done {
                    outputs: vec![PathBuf::from("a.png")],
                    finished_at: 5,
                }),
                job(JobStatus::Failed {
                    error: ImageError::new(ImageErrorKind::ContentPolicy, "no"),
                    finished_at: 6,
                }),
            ],
            ..StudioStore::default()
        };
        let json = serde_json::to_string(&store).unwrap();
        assert_eq!(serde_json::from_str::<StudioStore>(&json).unwrap(), store);
        // An older or partial file still loads.
        let partial: StudioStore = serde_json::from_str("{}").unwrap();
        assert!(partial.jobs.is_empty());
    }

    #[test]
    fn references_are_checked_before_they_are_copied() {
        let dir = std::env::temp_dir().join(format!("waku-studio-refs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let gif = dir.join("a.gif");
        std::fs::write(&gif, b"GIF8").unwrap();
        assert_eq!(
            copy_reference(&gif, &dir.join("copies")).unwrap_err(),
            ReferenceProblem::Format
        );
        let png = dir.join("b.png");
        std::fs::write(&png, [0x89, b'P', b'N', b'G']).unwrap();
        let (copy, _) = copy_reference(&png, &dir.join("copies")).unwrap();
        assert!(copy.starts_with(dir.join("copies")));
        assert_eq!(copy.extension().unwrap(), "png");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
