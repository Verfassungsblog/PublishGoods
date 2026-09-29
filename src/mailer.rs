use crate::settings::Settings;
use lettre::message::Mailbox;
use lettre::message::header::ContentType;
use lettre::transport::smtp::PoolConfig;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use std::collections::VecDeque;
use std::time::Duration;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::time::{Instant, timeout};

pub struct Mailer {
    pub sender: Sender<MailJob>,
}

pub struct MailJob {
    pub receiver: Mailbox,
    pub subject: String,
    pub body: String,
    pub content_type: ContentType,
}

impl MailJob {
    pub fn from(
        receiver: Mailbox,
        subject: String,
        body: String,
        content_type: ContentType,
    ) -> Self {
        MailJob {
            receiver,
            subject,
            body,
            content_type,
        }
    }
}

async fn send_email(
    transporter: &AsyncSmtpTransport<Tokio1Executor>,
    settings: &Settings,
    job: &MailJob,
) -> Result<(), String> {
    let from: Mailbox = settings
        .mail_from_address
        .parse()
        .map_err(|e| format!("Invalid from address configured: {}", e))?;

    let email = Message::builder()
        .from(from)
        .to(job.receiver.clone())
        .subject(&job.subject)
        .header(job.content_type.clone())
        .body(job.body.clone())
        .map_err(|e| format!("Couldn't build email: {}", e))?;

    transporter
        .send(email)
        .await
        .map_err(|e| format!("Couldn't send email: {}", e))?;
    Ok(())
}

fn init_smtp_pool(
    settings: &Settings,
) -> Result<AsyncSmtpTransport<Tokio1Executor>, lettre::transport::smtp::Error> {
    let pool_config = PoolConfig::new()
        .idle_timeout(Duration::from_secs(settings.smtp_pool_idle_timeout))
        .max_size(settings.smtp_pool_max_size)
        .min_idle(settings.smtp_pool_min_idle);
    Ok(
        AsyncSmtpTransport::<Tokio1Executor>::from_url(&settings.smtp_connection_url)?
            .pool_config(pool_config)
            .build(),
    )
}

struct MailToRetry {
    mail: MailJob,
    retry_count: u8,
    next_retry: Instant,
}

pub fn start_mail_worker(mut job_receiver: Receiver<MailJob>, settings: Settings) {
    tokio::spawn(async move {
        let transporter = match init_smtp_pool(&settings) {
            Ok(transport) => transport,
            Err(e) => {
                error!(
                    "Couldn't build smtp transporter: {}. Mailing will not be available!",
                    e
                );
                return;
            }
        };

        let mut retry_queue: VecDeque<MailToRetry> = VecDeque::new();
        let base_retry_delay = Duration::from_secs(settings.mail_base_retry_delay_seconds);

        loop {
            match timeout(Duration::from_millis(500), job_receiver.recv()).await {
                Ok(Some(job)) => {
                    if let Err(e) = send_email(&transporter, &settings, &job).await {
                        info!(
                            "Couldn't send email to {}: {}. Adding to retry queue.",
                            job.receiver, e
                        );
                        retry_queue.push_back(MailToRetry {
                            mail: job,
                            retry_count: 0,
                            next_retry: Instant::now() + base_retry_delay,
                        });
                    }
                }
                Ok(None) => {
                    // Sender was dropped, no more mail jobs will ever be received.
                    break;
                }
                Err(_) => {
                    // Timed out waiting for a new job, fall through to process the retry queue.
                }
            }

            let now = Instant::now();
            let queue_len = retry_queue.len();
            for _ in 0..queue_len {
                let Some(mut retry_job) = retry_queue.pop_front() else {
                    break;
                };

                if retry_job.next_retry > now {
                    retry_queue.push_back(retry_job);
                    continue;
                }

                if let Err(e) = send_email(&transporter, &settings, &retry_job.mail).await {
                    retry_job.retry_count += 1;
                    if retry_job.retry_count >= settings.mail_max_retries {
                        warn!(
                            "Giving up sending email to {} after {} retries: {}",
                            retry_job.mail.receiver, retry_job.retry_count, e
                        );
                    } else {
                        let delay = base_retry_delay * (retry_job.retry_count as u32 + 1);
                        retry_job.next_retry = now + delay;
                        info!(
                            "Retry {}/{} failed for email to {}: {}. Retrying again in {:?}.",
                            retry_job.retry_count,
                            settings.mail_max_retries,
                            retry_job.mail.receiver,
                            e,
                            delay
                        );
                        retry_queue.push_back(retry_job);
                    }
                }
            }
        }
    });
}
