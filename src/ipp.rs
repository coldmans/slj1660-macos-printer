use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Cursor;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::usb::{send_bytes, LibusbTransport, UsbTarget};

const DEFAULT_PRINTER_PATH: &str = "/printers/slj1660";
const IPP_PRINT_JOB: u16 = 0x0002;
const IPP_VALIDATE_JOB: u16 = 0x0004;
const IPP_CREATE_JOB: u16 = 0x0005;
const IPP_SEND_DOCUMENT: u16 = 0x0006;
const IPP_CANCEL_JOB: u16 = 0x0008;
const IPP_GET_JOB_ATTRIBUTES: u16 = 0x0009;
const IPP_GET_JOBS: u16 = 0x000a;
const IPP_GET_PRINTER_ATTRIBUTES: u16 = 0x000b;

const STATUS_OK: u16 = 0x0000;
const STATUS_BAD_REQUEST: u16 = 0x0400;
const STATUS_NOT_POSSIBLE: u16 = 0x0404;
const STATUS_DOCUMENT_FORMAT_NOT_SUPPORTED: u16 = 0x040a;

const JOB_STATE_PENDING: i32 = 3;
const JOB_STATE_PROCESSING: i32 = 5;
const JOB_STATE_CANCELED: i32 = 7;
const JOB_STATE_ABORTED: i32 = 8;
const JOB_STATE_COMPLETED: i32 = 9;

const GROUP_OPERATION_ATTRIBUTES: u8 = 0x01;
const GROUP_JOB_ATTRIBUTES: u8 = 0x02;
const GROUP_END: u8 = 0x03;
const GROUP_PRINTER_ATTRIBUTES: u8 = 0x04;

const TAG_INTEGER: u8 = 0x21;
const TAG_BOOLEAN: u8 = 0x22;
const TAG_ENUM: u8 = 0x23;
const TAG_TEXT: u8 = 0x41;
const TAG_NAME: u8 = 0x42;
const TAG_KEYWORD: u8 = 0x44;
const TAG_URI: u8 = 0x45;
const TAG_CHARSET: u8 = 0x47;
const TAG_NATURAL_LANGUAGE: u8 = 0x48;
const TAG_MIME_MEDIA_TYPE: u8 = 0x49;
const AUTO_RESUME_INITIAL_DELAY: Duration = Duration::from_secs(12);
const AUTO_RESUME_REPEAT_DELAY: Duration = Duration::from_secs(15);
const AUTO_RESUME_MAX_ATTEMPTS: usize = 3;

static NEXT_JOB_ID: AtomicU32 = AtomicU32::new(1);

#[derive(Debug, Clone)]
pub struct IppServerConfig {
    pub bind: SocketAddr,
    pub printer_path: String,
    pub project_root: PathBuf,
    pub script_path: PathBuf,
    pub spool_dir: PathBuf,
    pub serial_number: Option<String>,
    pub dry_run: bool,
    pub confirm_alerts: bool,
    pub chunk_size: usize,
    pub chunk_delay: Duration,
    pub timeout: Duration,
    pub max_pages: u32,
}

impl IppServerConfig {
    pub fn printer_uri(&self) -> String {
        format!("ipp://{}{}", self.bind, self.printer_path)
    }
}

#[derive(Debug, Clone)]
struct IppServerState {
    config: IppServerConfig,
    jobs: Arc<Mutex<HashMap<u32, StoredJob>>>,
}

impl IppServerState {
    fn new(config: IppServerConfig) -> Self {
        Self {
            config,
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn insert_job(&self, job: StoredJob) {
        let mut jobs = self.jobs.lock().expect("IPP job store lock poisoned");
        jobs.insert(job.id, job);
    }

    fn update_job(&self, job: StoredJob) {
        self.insert_job(job);
    }

    fn get_job(&self, job_id: u32) -> Option<StoredJob> {
        let jobs = self.jobs.lock().expect("IPP job store lock poisoned");
        jobs.get(&job_id).cloned()
    }
}

pub fn default_printer_path() -> String {
    DEFAULT_PRINTER_PATH.to_string()
}

pub fn default_project_root() -> PathBuf {
    std::env::var_os("SLJ1660_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

pub fn default_script_path(project_root: &Path) -> PathBuf {
    project_root.join("scripts/print-pdf-mode10.py")
}

pub fn default_spool_dir() -> PathBuf {
    std::env::temp_dir().join("slj1660-ipp-spool")
}

pub fn serve_ipp(config: IppServerConfig) -> Result<()> {
    validate_config(&config)?;
    fs::create_dir_all(&config.spool_dir)
        .with_context(|| format!("failed to create spool dir {}", config.spool_dir.display()))?;

    let server = Server::http(config.bind)
        .map_err(|error| anyhow!("failed to bind IPP server on {}: {error}", config.bind))?;
    let state = IppServerState::new(config);

    println!(
        "SL-J1660 local IPP printer app listening on {}",
        state.config.bind
    );
    println!("Printer URI: {}", state.config.printer_uri());
    if state.config.dry_run {
        println!("Dry-run mode: jobs will be rendered to raw files but not sent to USB.");
    } else {
        println!("Print mode: accepted jobs can physically print and consume ink/paper.");
    }

    loop {
        let request = server.recv().context("failed to accept HTTP request")?;
        if let Err(error) = handle_http_request(request, &state) {
            eprintln!("IPP request failed: {error:#}");
        }
    }
}

fn validate_config(config: &IppServerConfig) -> Result<()> {
    if config.printer_path.is_empty() || !config.printer_path.starts_with('/') {
        bail!("printer path must start with /");
    }
    if config.chunk_size == 0 {
        bail!("chunk size must be greater than zero");
    }
    if config.max_pages == 0 {
        bail!("max pages must be greater than zero");
    }
    if !config.script_path.exists() {
        bail!(
            "Mode10 PDF script does not exist: {}",
            config.script_path.display()
        );
    }
    Ok(())
}

fn handle_http_request(mut request: Request, state: &IppServerState) -> Result<()> {
    let method = request.method().clone();
    let url = request.url().to_string();
    let config = &state.config;

    match (method, url.as_str()) {
        (Method::Get, "/") | (Method::Get, "/health") => {
            let body = format!(
                "SL-J1660 local IPP printer app\nprinter_uri={}\ndry_run={}\n",
                config.printer_uri(),
                config.dry_run
            );
            request.respond(text_response(StatusCode(200), body))?;
        }
        (Method::Post, path) if path == config.printer_path => {
            let mut body = Vec::new();
            request
                .as_reader()
                .read_to_end(&mut body)
                .context("failed to read IPP request body")?;
            let response = handle_ipp_body(&body, state);
            request.respond(response_to_http(response))?;
        }
        _ => {
            request.respond(text_response(
                StatusCode(404),
                format!("unknown SL-J1660 printer app path: {url}\n"),
            ))?;
        }
    }
    Ok(())
}

fn response_to_http(response: IppResponse) -> Response<Cursor<Vec<u8>>> {
    let mut http = Response::from_data(response.bytes).with_status_code(StatusCode(response.http));
    http.add_header(ipp_content_type_header());
    http
}

fn text_response(status: StatusCode, body: String) -> Response<Cursor<Vec<u8>>> {
    let mut response = Response::from_string(body).with_status_code(status);
    response.add_header(text_content_type_header());
    response
}

fn ipp_content_type_header() -> Header {
    Header::from_bytes(&b"Content-Type"[..], &b"application/ipp"[..])
        .expect("static IPP content-type header is valid")
}

fn text_content_type_header() -> Header {
    Header::from_bytes(&b"Content-Type"[..], &b"text/plain; charset=utf-8"[..])
        .expect("static text content-type header is valid")
}

#[derive(Debug, Clone)]
struct IppResponse {
    http: u16,
    bytes: Vec<u8>,
}

fn handle_ipp_body(bytes: &[u8], state: &IppServerState) -> IppResponse {
    let parsed = match IppRequest::parse(bytes) {
        Ok(parsed) => parsed,
        Err(error) => {
            return simple_ipp_response(1, 1, STATUS_BAD_REQUEST, 1, Some(error.to_string()));
        }
    };

    println!(
        "IPP operation 0x{:04x}, request {}, document {} byte(s)",
        parsed.operation_id,
        parsed.request_id,
        parsed.document.len()
    );

    match parsed.operation_id {
        IPP_GET_PRINTER_ATTRIBUTES => get_printer_attributes_response(&parsed, &state.config),
        IPP_VALIDATE_JOB => simple_ipp_response(
            parsed.version_major,
            parsed.version_minor,
            STATUS_OK,
            parsed.request_id,
            Some("SL-J1660 local printer app can accept this job".to_string()),
        ),
        IPP_CREATE_JOB => create_job_response(&parsed, state),
        IPP_SEND_DOCUMENT => send_document_response(&parsed, state),
        IPP_CANCEL_JOB => cancel_job_response(&parsed, state),
        IPP_GET_JOBS => empty_jobs_response(&parsed),
        IPP_GET_JOB_ATTRIBUTES => get_job_attributes_response(&parsed, state),
        IPP_PRINT_JOB => print_job_response(&parsed, state),
        operation => simple_ipp_response(
            parsed.version_major,
            parsed.version_minor,
            STATUS_NOT_POSSIBLE,
            parsed.request_id,
            Some(format!(
                "operation 0x{operation:04x} is not implemented by this MVP"
            )),
        ),
    }
}

fn simple_ipp_response(
    version_major: u8,
    version_minor: u8,
    status: u16,
    request_id: u32,
    message: Option<String>,
) -> IppResponse {
    let mut builder = IppResponseBuilder::new(version_major, version_minor, status, request_id);
    builder.operation_attributes();
    if let Some(message) = message {
        builder.text(TAG_TEXT, "status-message", &message);
    }
    builder.finish();
    IppResponse {
        http: 200,
        bytes: builder.into_bytes(),
    }
}

fn get_printer_attributes_response(request: &IppRequest, config: &IppServerConfig) -> IppResponse {
    let mut builder = IppResponseBuilder::new(
        request.version_major,
        request.version_minor,
        STATUS_OK,
        request.request_id,
    );
    builder.operation_attributes();
    builder.group(GROUP_PRINTER_ATTRIBUTES);
    builder.text(TAG_URI, "printer-uri-supported", &config.printer_uri());
    builder.text(TAG_NAME, "printer-name", "Samsung SL-J1660 Local");
    builder.text(
        TAG_TEXT,
        "printer-info",
        "Samsung SL-J1660 via local Mode10 USB printer app",
    );
    builder.text(
        TAG_TEXT,
        "printer-make-and-model",
        "Samsung SL-J1660 Series",
    );
    builder.text(TAG_KEYWORD, "uri-authentication-supported", "none");
    builder.text(TAG_KEYWORD, "uri-security-supported", "none");
    builder.text(TAG_CHARSET, "charset-configured", "utf-8");
    builder.text_values(TAG_CHARSET, "charset-supported", &["utf-8"]);
    builder.text(TAG_NATURAL_LANGUAGE, "natural-language-configured", "en");
    builder.text_values(
        TAG_NATURAL_LANGUAGE,
        "generated-natural-language-supported",
        &["en"],
    );
    builder.text_values(TAG_KEYWORD, "ipp-versions-supported", &["1.1", "2.0"]);
    builder.integer_values(
        TAG_ENUM,
        "operations-supported",
        &[
            IPP_PRINT_JOB as i32,
            IPP_VALIDATE_JOB as i32,
            IPP_CREATE_JOB as i32,
            IPP_SEND_DOCUMENT as i32,
            IPP_CANCEL_JOB as i32,
            IPP_GET_JOB_ATTRIBUTES as i32,
            IPP_GET_JOBS as i32,
            IPP_GET_PRINTER_ATTRIBUTES as i32,
        ],
    );
    builder.text_values(
        TAG_MIME_MEDIA_TYPE,
        "document-format-supported",
        &["application/pdf"],
    );
    builder.text(
        TAG_MIME_MEDIA_TYPE,
        "document-format-default",
        "application/pdf",
    );
    builder.text(TAG_KEYWORD, "compression-supported", "none");
    builder.boolean("printer-is-accepting-jobs", true);
    builder.integer(TAG_ENUM, "printer-state", 3);
    builder.text(TAG_KEYWORD, "printer-state-reasons", "none");
    builder.integer(TAG_INTEGER, "queued-job-count", 0);
    builder.boolean("color-supported", false);
    builder.text_values(TAG_KEYWORD, "sides-supported", &["one-sided"]);
    builder.text(TAG_KEYWORD, "sides-default", "one-sided");
    builder.text_values(
        TAG_KEYWORD,
        "media-supported",
        &["iso_a4_210x297mm", "na_letter_8.5x11in"],
    );
    builder.text(TAG_KEYWORD, "media-default", "iso_a4_210x297mm");
    builder.text_values(TAG_KEYWORD, "print-color-mode-supported", &["monochrome"]);
    builder.text(TAG_KEYWORD, "print-color-mode-default", "monochrome");
    builder.integer_values(TAG_ENUM, "print-quality-supported", &[3, 4, 5]);
    builder.integer(TAG_ENUM, "print-quality-default", 4);
    builder.text(TAG_KEYWORD, "pdl-override-supported", "not-attempted");
    builder.finish();

    IppResponse {
        http: 200,
        bytes: builder.into_bytes(),
    }
}

fn empty_jobs_response(request: &IppRequest) -> IppResponse {
    let mut builder = IppResponseBuilder::new(
        request.version_major,
        request.version_minor,
        STATUS_OK,
        request.request_id,
    );
    builder.operation_attributes();
    builder.finish();
    IppResponse {
        http: 200,
        bytes: builder.into_bytes(),
    }
}

fn create_job_response(request: &IppRequest, state: &IppServerState) -> IppResponse {
    let job_id = NEXT_JOB_ID.fetch_add(1, Ordering::Relaxed);
    let job = StoredJob::pending(job_id, "job created; waiting for document bytes");
    state.insert_job(job.clone());
    job_attributes_response(request, &state.config, job)
}

fn send_document_response(request: &IppRequest, state: &IppServerState) -> IppResponse {
    let Some(job_id) = request.job_id() else {
        return simple_ipp_response(
            request.version_major,
            request.version_minor,
            STATUS_BAD_REQUEST,
            request.request_id,
            Some("Send-Document requires a job-id or job-uri attribute".to_string()),
        );
    };

    document_job_response(request, state, job_id)
}

fn print_job_response(request: &IppRequest, state: &IppServerState) -> IppResponse {
    let job_id = NEXT_JOB_ID.fetch_add(1, Ordering::Relaxed);
    document_job_response(request, state, job_id)
}

fn document_job_response(request: &IppRequest, state: &IppServerState, job_id: u32) -> IppResponse {
    if request.document.is_empty() {
        return simple_ipp_response(
            request.version_major,
            request.version_minor,
            STATUS_BAD_REQUEST,
            request.request_id,
            Some("Print-Job did not include document bytes".to_string()),
        );
    }

    if !looks_like_pdf(&request.document) {
        return simple_ipp_response(
            request.version_major,
            request.version_minor,
            STATUS_DOCUMENT_FORMAT_NOT_SUPPORTED,
            request.request_id,
            Some(
                "this MVP currently accepts PDF document bytes only; configure the queue as driverless/everywhere"
                    .to_string(),
            ),
        );
    }

    let job = StoredJob::processing(job_id, "document accepted; rendering in background");
    state.insert_job(job.clone());

    let background_state = state.clone();
    let document = request.document.clone();
    thread::spawn(move || {
        let result = run_pdf_job(&background_state.config, job_id, &document);
        match result {
            Ok(summary) => {
                background_state.update_job(StoredJob::completed(
                    job_id,
                    summary,
                    background_state.config.dry_run,
                ));
                println!("SL-J1660 job {job_id} completed");
            }
            Err(error) => {
                eprintln!("SL-J1660 job {job_id} failed: {error:#}");
                background_state.update_job(StoredJob::aborted(
                    job_id,
                    format!("SL-J1660 job failed: {error:#}"),
                ));
            }
        }
    });

    job_attributes_response(request, &state.config, job)
}

fn cancel_job_response(request: &IppRequest, state: &IppServerState) -> IppResponse {
    let Some(job_id) = request.job_id() else {
        return simple_ipp_response(
            request.version_major,
            request.version_minor,
            STATUS_BAD_REQUEST,
            request.request_id,
            Some("Cancel-Job requires a job-id or job-uri attribute".to_string()),
        );
    };

    let job = StoredJob::canceled(job_id, "job canceled by IPP client");
    state.insert_job(job.clone());
    job_attributes_response(request, &state.config, job)
}

fn get_job_attributes_response(request: &IppRequest, state: &IppServerState) -> IppResponse {
    let job_id = request.job_id().unwrap_or(1);
    let job = state
        .get_job(job_id)
        .unwrap_or_else(|| StoredJob::unknown(job_id));
    job_attributes_response(request, &state.config, job)
}

fn job_attributes_response(
    request: &IppRequest,
    config: &IppServerConfig,
    job: StoredJob,
) -> IppResponse {
    let mut builder = IppResponseBuilder::new(
        request.version_major,
        request.version_minor,
        STATUS_OK,
        request.request_id,
    );
    builder.operation_attributes();
    builder.group(GROUP_JOB_ATTRIBUTES);
    builder.integer(TAG_INTEGER, "job-id", job.id as i32);
    builder.text(
        TAG_URI,
        "job-uri",
        &format!("{}/jobs/{}", config.printer_uri(), job.id),
    );
    builder.integer(TAG_ENUM, "job-state", job.state);
    builder.text(TAG_KEYWORD, "job-state-reasons", job.reason());
    builder.text(TAG_TEXT, "job-state-message", &job.message);
    builder.integer(TAG_INTEGER, "job-media-sheets-completed", job.pages as i32);
    builder.integer(
        TAG_INTEGER,
        "job-k-octets-processed",
        job.raw_bytes.div_ceil(1024).min(i32::MAX as u64) as i32,
    );
    builder.finish();

    IppResponse {
        http: 200,
        bytes: builder.into_bytes(),
    }
}

#[derive(Debug, Clone)]
struct PrintJobSummary {
    pages: u32,
    raw_bytes: u64,
}

#[derive(Debug, Clone)]
struct StoredJob {
    id: u32,
    state: i32,
    message: String,
    pages: u32,
    raw_bytes: u64,
}

impl StoredJob {
    fn pending(id: u32, message: impl Into<String>) -> Self {
        Self {
            id,
            state: JOB_STATE_PENDING,
            message: message.into(),
            pages: 0,
            raw_bytes: 0,
        }
    }

    fn processing(id: u32, message: impl Into<String>) -> Self {
        Self {
            id,
            state: JOB_STATE_PROCESSING,
            message: message.into(),
            pages: 0,
            raw_bytes: 0,
        }
    }

    fn completed(id: u32, summary: PrintJobSummary, dry_run: bool) -> Self {
        let suffix = if dry_run { " in dry-run mode" } else { "" };
        Self {
            id,
            state: JOB_STATE_COMPLETED,
            message: format!(
                "completed {} page(s), {} raw byte(s){}",
                summary.pages, summary.raw_bytes, suffix
            ),
            pages: summary.pages,
            raw_bytes: summary.raw_bytes,
        }
    }

    fn aborted(id: u32, message: impl Into<String>) -> Self {
        Self {
            id,
            state: JOB_STATE_ABORTED,
            message: message.into(),
            pages: 0,
            raw_bytes: 0,
        }
    }

    fn canceled(id: u32, message: impl Into<String>) -> Self {
        Self {
            id,
            state: JOB_STATE_CANCELED,
            message: message.into(),
            pages: 0,
            raw_bytes: 0,
        }
    }

    fn unknown(id: u32) -> Self {
        Self {
            id,
            state: JOB_STATE_COMPLETED,
            message: "job history is not persisted by this MVP".to_string(),
            pages: 0,
            raw_bytes: 0,
        }
    }

    fn reason(&self) -> &'static str {
        match self.state {
            JOB_STATE_CANCELED => "job-canceled-by-user",
            JOB_STATE_ABORTED => "aborted-by-system",
            JOB_STATE_PROCESSING => "job-printing",
            _ => "none",
        }
    }
}

fn run_pdf_job(config: &IppServerConfig, job_id: u32, pdf_bytes: &[u8]) -> Result<PrintJobSummary> {
    let job_dir = config.spool_dir.join(format!("job-{job_id:06}"));
    fs::create_dir_all(&job_dir)
        .with_context(|| format!("failed to create job dir {}", job_dir.display()))?;

    let pdf_path = job_dir.join("input.pdf");
    fs::write(&pdf_path, pdf_bytes)
        .with_context(|| format!("failed to write {}", pdf_path.display()))?;

    let page_count = detect_pdf_page_count(&pdf_path)
        .unwrap_or(1)
        .clamp(1, config.max_pages);
    let mut raw_bytes = 0;

    if config.confirm_alerts && !config.dry_run {
        if let Err(error) = send_preflight_confirmations(config) {
            eprintln!("warning: preflight LEDM confirmation failed: {error:#}");
        }
    }

    for page in 1..=page_count {
        let raw_path = job_dir.join(format!("page-{page}.raw"));
        generate_page_raw(config, &pdf_path, &raw_path, page)?;
        raw_bytes += fs::metadata(&raw_path)
            .with_context(|| format!("failed to stat {}", raw_path.display()))?
            .len();

        if !config.dry_run {
            send_raw_to_printer(config, &raw_path)
                .with_context(|| format!("failed to send page {page} raw stream"))?;
        }
    }

    if config.confirm_alerts && !config.dry_run {
        if let Err(error) = send_postflight_confirmation(config) {
            eprintln!("warning: postflight LEDM confirmation failed: {error:#}");
        }
    }

    Ok(PrintJobSummary {
        pages: page_count,
        raw_bytes,
    })
}

fn detect_pdf_page_count(pdf_path: &Path) -> Option<u32> {
    let output = Command::new("pdfinfo")
        .env("PATH", child_process_path())
        .arg(pdf_path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().find_map(|line| {
        line.strip_prefix("Pages:")
            .and_then(|value| value.trim().parse::<u32>().ok())
    })
}

fn generate_page_raw(
    config: &IppServerConfig,
    pdf_path: &Path,
    raw_path: &Path,
    page: u32,
) -> Result<()> {
    let python = python_executable();
    let output = Command::new(&python)
        .arg(&config.script_path)
        .arg(pdf_path)
        .arg("--out")
        .arg(raw_path)
        .arg("--page")
        .arg(page.to_string())
        .env("PATH", child_process_path())
        .current_dir(&config.project_root)
        .output()
        .with_context(|| format!("failed to run {}", config.script_path.display()))?;

    if !output.status.success() {
        bail!(
            "Mode10 PDF script failed with status {} using Python {}:\nstdout:\n{}\nstderr:\n{}",
            output.status,
            python.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    if !raw_path.exists() {
        bail!("Mode10 PDF script did not create {}", raw_path.display());
    }
    Ok(())
}

fn python_executable() -> PathBuf {
    env::var_os("SLJ1660_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("python3"))
}

fn child_process_path() -> String {
    let mut parts = vec![
        "/opt/homebrew/bin".to_string(),
        "/usr/local/bin".to_string(),
        "/usr/bin".to_string(),
        "/bin".to_string(),
        "/usr/sbin".to_string(),
        "/sbin".to_string(),
    ];

    if let Some(path) = env::var_os("PATH").and_then(|path| path.into_string().ok()) {
        parts.extend(path.split(':').map(str::to_string));
    }

    let mut deduped = Vec::new();
    for part in parts {
        if !part.is_empty() && !deduped.contains(&part) {
            deduped.push(part);
        }
    }
    deduped.join(":")
}

fn send_raw_to_printer(config: &IppServerConfig, raw_path: &Path) -> Result<()> {
    let bytes =
        fs::read(raw_path).with_context(|| format!("failed to read {}", raw_path.display()))?;
    let transfer_done = if config.confirm_alerts {
        Some(spawn_auto_resume_watchdog(
            config.clone(),
            raw_path.to_path_buf(),
        ))
    } else {
        None
    };
    let mut transport = LibusbTransport::open(
        UsbTarget {
            serial_number: config.serial_number.clone(),
            ..UsbTarget::default()
        },
        config.timeout,
        config.chunk_size,
    )?;
    transport.set_chunk_delay(config.chunk_delay);
    let result = send_bytes(&mut transport, &bytes);
    if let Some(done) = transfer_done {
        done.store(true, Ordering::Relaxed);
    }
    result?;
    Ok(())
}

fn spawn_auto_resume_watchdog(config: IppServerConfig, raw_path: PathBuf) -> Arc<AtomicBool> {
    let done = Arc::new(AtomicBool::new(false));
    let thread_done = Arc::clone(&done);
    thread::spawn(move || {
        thread::sleep(AUTO_RESUME_INITIAL_DELAY);

        for attempt in 1..=AUTO_RESUME_MAX_ATTEMPTS {
            if thread_done.load(Ordering::Relaxed) {
                return;
            }

            let elapsed = AUTO_RESUME_INITIAL_DELAY
                + AUTO_RESUME_REPEAT_DELAY * u32::try_from(attempt - 1).unwrap_or(0);
            eprintln!(
                "warning: {} is still transferring after {:?}; sending LEDM resume attempt {attempt}/{}",
                raw_path.display(),
                elapsed,
                AUTO_RESUME_MAX_ATTEMPTS
            );

            match send_feed_attention_resume(&config) {
                Ok(()) => eprintln!("warning: LEDM feed-attention resume attempt {attempt} sent"),
                Err(error) => {
                    eprintln!(
                        "warning: LEDM feed-attention resume attempt {attempt} failed: {error:#}"
                    )
                }
            }

            thread::sleep(AUTO_RESUME_REPEAT_DELAY);
        }
    });
    done
}

fn send_preflight_confirmations(config: &IppServerConfig) -> Result<()> {
    send_ledm_fixture(config, "fixtures/confirm/lowink-continue.http", None)?;
    send_ledm_fixture(config, "fixtures/confirm/cartridge-refilled-ok.http", None)?;
    Ok(())
}

fn send_postflight_confirmation(config: &IppServerConfig) -> Result<()> {
    send_ledm_fixture(
        config,
        "fixtures/confirm/single-cartridge-ok.http",
        Some(1024),
    )
}

fn send_feed_attention_resume(config: &IppServerConfig) -> Result<()> {
    send_ledm_fixture(
        config,
        "fixtures/confirm/tray-empty-or-open-resume.http",
        Some(861),
    )
}

fn send_ledm_fixture(
    config: &IppServerConfig,
    relative_path: &str,
    chunk_size: Option<usize>,
) -> Result<()> {
    let path = config.project_root.join(relative_path);
    let bytes = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut transport = LibusbTransport::open(
        UsbTarget {
            serial_number: config.serial_number.clone(),
            interface_number: Some(3),
            endpoint_address: Some(0x0a),
            ..UsbTarget::default()
        },
        config.timeout,
        chunk_size.unwrap_or(bytes.len()),
    )?;
    send_bytes(&mut transport, &bytes)?;
    Ok(())
}

fn looks_like_pdf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%PDF")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IppRequest {
    version_major: u8,
    version_minor: u8,
    operation_id: u16,
    request_id: u32,
    attributes: Vec<IppAttribute>,
    document: Vec<u8>,
}

impl IppRequest {
    fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 8 {
            bail!("IPP request is too short");
        }

        let version_major = bytes[0];
        let version_minor = bytes[1];
        let operation_id = u16::from_be_bytes([bytes[2], bytes[3]]);
        let request_id = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);

        let mut offset = 8;
        let mut group_tag = None;
        let mut last_name = String::new();
        let mut attributes = Vec::new();

        while offset < bytes.len() {
            let tag = bytes[offset];
            offset += 1;

            if tag == GROUP_END {
                return Ok(Self {
                    version_major,
                    version_minor,
                    operation_id,
                    request_id,
                    attributes,
                    document: bytes[offset..].to_vec(),
                });
            }

            if tag <= 0x0f {
                group_tag = Some(tag);
                last_name.clear();
                continue;
            }

            if offset + 2 > bytes.len() {
                bail!("IPP attribute is missing name length");
            }
            let name_len = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]) as usize;
            offset += 2;
            if offset + name_len > bytes.len() {
                bail!("IPP attribute name is truncated");
            }
            let name = if name_len == 0 {
                last_name.clone()
            } else {
                let parsed = String::from_utf8_lossy(&bytes[offset..offset + name_len]).to_string();
                last_name = parsed.clone();
                parsed
            };
            offset += name_len;

            if offset + 2 > bytes.len() {
                bail!("IPP attribute is missing value length");
            }
            let value_len = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]) as usize;
            offset += 2;
            if offset + value_len > bytes.len() {
                bail!("IPP attribute value is truncated");
            }
            let value = bytes[offset..offset + value_len].to_vec();
            offset += value_len;

            attributes.push(IppAttribute {
                group_tag: group_tag.unwrap_or(GROUP_OPERATION_ATTRIBUTES),
                value_tag: tag,
                name,
                value,
            });
        }

        bail!("IPP request is missing end-of-attributes tag")
    }

    fn attribute(&self, name: &str) -> Option<&IppAttribute> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name == name)
    }

    fn integer_attribute(&self, name: &str) -> Option<i32> {
        let attribute = self.attribute(name)?;
        (attribute.value.len() == 4).then(|| {
            i32::from_be_bytes([
                attribute.value[0],
                attribute.value[1],
                attribute.value[2],
                attribute.value[3],
            ])
        })
    }

    fn text_attribute(&self, name: &str) -> Option<String> {
        self.attribute(name)
            .map(|attribute| String::from_utf8_lossy(&attribute.value).to_string())
    }

    fn job_id(&self) -> Option<u32> {
        self.integer_attribute("job-id")
            .and_then(|value| u32::try_from(value).ok())
            .or_else(|| {
                self.text_attribute("job-uri")
                    .and_then(|uri| uri.rsplit('/').next().and_then(|id| id.parse().ok()))
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IppAttribute {
    group_tag: u8,
    value_tag: u8,
    name: String,
    value: Vec<u8>,
}

#[derive(Debug, Clone)]
struct IppResponseBuilder {
    bytes: Vec<u8>,
}

impl IppResponseBuilder {
    fn new(version_major: u8, version_minor: u8, status: u16, request_id: u32) -> Self {
        let mut bytes = Vec::new();
        bytes.push(version_major);
        bytes.push(version_minor);
        bytes.extend_from_slice(&status.to_be_bytes());
        bytes.extend_from_slice(&request_id.to_be_bytes());
        Self { bytes }
    }

    fn operation_attributes(&mut self) {
        self.group(GROUP_OPERATION_ATTRIBUTES);
        self.text(TAG_CHARSET, "attributes-charset", "utf-8");
        self.text(TAG_NATURAL_LANGUAGE, "attributes-natural-language", "en");
    }

    fn group(&mut self, tag: u8) {
        self.bytes.push(tag);
    }

    fn text(&mut self, value_tag: u8, name: &str, value: &str) {
        self.attribute(value_tag, name, value.as_bytes());
    }

    fn text_values(&mut self, value_tag: u8, name: &str, values: &[&str]) {
        for (index, value) in values.iter().enumerate() {
            self.attribute(
                value_tag,
                if index == 0 { name } else { "" },
                value.as_bytes(),
            );
        }
    }

    fn integer(&mut self, value_tag: u8, name: &str, value: i32) {
        self.attribute(value_tag, name, &value.to_be_bytes());
    }

    fn integer_values(&mut self, value_tag: u8, name: &str, values: &[i32]) {
        for (index, value) in values.iter().enumerate() {
            self.attribute(
                value_tag,
                if index == 0 { name } else { "" },
                &value.to_be_bytes(),
            );
        }
    }

    fn boolean(&mut self, name: &str, value: bool) {
        self.attribute(TAG_BOOLEAN, name, &[u8::from(value)]);
    }

    fn attribute(&mut self, value_tag: u8, name: &str, value: &[u8]) {
        self.bytes.push(value_tag);
        self.bytes
            .extend_from_slice(&(name.len() as u16).to_be_bytes());
        self.bytes.extend_from_slice(name.as_bytes());
        self.bytes
            .extend_from_slice(&(value.len() as u16).to_be_bytes());
        self.bytes.extend_from_slice(value);
    }

    fn finish(&mut self) {
        self.bytes.push(GROUP_END);
    }

    fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_print_job_document_after_end_tag() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x02, 0x00]);
        bytes.extend_from_slice(&IPP_PRINT_JOB.to_be_bytes());
        bytes.extend_from_slice(&42_u32.to_be_bytes());
        bytes.push(GROUP_OPERATION_ATTRIBUTES);
        push_text_attr(&mut bytes, TAG_CHARSET, "attributes-charset", "utf-8");
        bytes.push(GROUP_END);
        bytes.extend_from_slice(b"%PDF-1.4\n");

        let parsed = IppRequest::parse(&bytes).unwrap();
        assert_eq!(parsed.version_major, 2);
        assert_eq!(parsed.operation_id, IPP_PRINT_JOB);
        assert_eq!(parsed.request_id, 42);
        assert_eq!(parsed.document, b"%PDF-1.4\n");
        assert_eq!(parsed.attributes[0].name, "attributes-charset");
    }

    #[test]
    fn printer_attributes_response_contains_pdf_support() {
        let config = test_config();
        let request = IppRequest {
            version_major: 2,
            version_minor: 0,
            operation_id: IPP_GET_PRINTER_ATTRIBUTES,
            request_id: 7,
            attributes: Vec::new(),
            document: Vec::new(),
        };

        let response = get_printer_attributes_response(&request, &config);
        assert_eq!(&response.bytes[..4], &[0x02, 0x00, 0x00, 0x00]);
        assert!(response
            .bytes
            .windows(b"application/pdf".len())
            .any(|window| window == b"application/pdf"));
    }

    #[test]
    fn print_job_without_pdf_is_rejected() {
        let state = IppServerState::new(test_config());
        let request = IppRequest {
            version_major: 2,
            version_minor: 0,
            operation_id: IPP_PRINT_JOB,
            request_id: 99,
            attributes: Vec::new(),
            document: b"not a pdf".to_vec(),
        };

        let response = print_job_response(&request, &state);
        assert_eq!(&response.bytes[..4], &[0x02, 0x00, 0x04, 0x0a]);
    }

    #[test]
    fn create_job_stores_pending_job() {
        let state = IppServerState::new(test_config());
        let request = IppRequest {
            version_major: 2,
            version_minor: 0,
            operation_id: IPP_CREATE_JOB,
            request_id: 100,
            attributes: Vec::new(),
            document: Vec::new(),
        };

        let response = create_job_response(&request, &state);
        assert_eq!(&response.bytes[..4], &[0x02, 0x00, 0x00, 0x00]);
        let jobs = state.jobs.lock().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs.values().next().unwrap().state, JOB_STATE_PENDING);
    }

    #[test]
    fn print_job_returns_before_background_processing_finishes() {
        let state = IppServerState::new(test_config());
        let request = IppRequest {
            version_major: 2,
            version_minor: 0,
            operation_id: IPP_PRINT_JOB,
            request_id: 103,
            attributes: Vec::new(),
            document: b"%PDF-1.4\n".to_vec(),
        };

        let response = print_job_response(&request, &state);
        assert_eq!(&response.bytes[..4], &[0x02, 0x00, 0x00, 0x00]);
        assert!(response
            .bytes
            .windows(b"document accepted".len())
            .any(|window| window == b"document accepted"));
    }

    #[test]
    fn parses_job_id_from_integer_or_job_uri() {
        let request = IppRequest {
            version_major: 2,
            version_minor: 0,
            operation_id: IPP_SEND_DOCUMENT,
            request_id: 101,
            attributes: vec![IppAttribute {
                group_tag: GROUP_OPERATION_ATTRIBUTES,
                value_tag: TAG_INTEGER,
                name: "job-id".to_string(),
                value: 123_i32.to_be_bytes().to_vec(),
            }],
            document: Vec::new(),
        };
        assert_eq!(request.job_id(), Some(123));

        let request = IppRequest {
            version_major: 2,
            version_minor: 0,
            operation_id: IPP_SEND_DOCUMENT,
            request_id: 102,
            attributes: vec![IppAttribute {
                group_tag: GROUP_OPERATION_ATTRIBUTES,
                value_tag: TAG_URI,
                name: "job-uri".to_string(),
                value: b"ipp://127.0.0.1:8631/printers/slj1660/jobs/456".to_vec(),
            }],
            document: Vec::new(),
        };
        assert_eq!(request.job_id(), Some(456));
    }

    fn push_text_attr(bytes: &mut Vec<u8>, value_tag: u8, name: &str, value: &str) {
        bytes.push(value_tag);
        bytes.extend_from_slice(&(name.len() as u16).to_be_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(&(value.len() as u16).to_be_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }

    fn test_config() -> IppServerConfig {
        IppServerConfig {
            bind: "127.0.0.1:8631".parse().unwrap(),
            printer_path: default_printer_path(),
            project_root: PathBuf::from("/tmp/slj1660"),
            script_path: PathBuf::from("/tmp/slj1660/scripts/print-pdf-mode10.py"),
            spool_dir: PathBuf::from("/tmp/slj1660-spool"),
            serial_number: Some("TEST".to_string()),
            dry_run: true,
            confirm_alerts: false,
            chunk_size: 1024,
            chunk_delay: Duration::ZERO,
            timeout: Duration::from_secs(1),
            max_pages: 1,
        }
    }
}
