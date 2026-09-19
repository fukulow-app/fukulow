use domain::{ActorId, OrganizationId};
use sqlx::{PgPool, types::Uuid};
use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

pub(crate) struct Answers {
    pub(crate) organization_name: String,
    pub(crate) slug: String,
    pub(crate) display_name: String,
    pub(crate) email: String,
    pub(crate) password: String,
    pub(crate) confirmation: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    pub(crate) message: &'static str,
    pub(crate) code: u8,
}

impl Failure {
    fn field(message: &'static str) -> Self {
        Self { message, code: 1 }
    }
    fn generic() -> Self {
        Self::field("Bootstrap failed")
    }
}

impl From<db::BootstrapError> for Failure {
    fn from(error: db::BootstrapError) -> Self {
        match error {
            db::BootstrapError::AlreadyBootstrapped => Self {
                message: "The installation is already bootstrapped",
                code: 2,
            },
            db::BootstrapError::InvalidSlug => Self::field("Invalid organization slug"),
            db::BootstrapError::EmailTaken => Self::field("Email is already registered"),
            db::BootstrapError::Database(_) => Self::generic(),
        }
    }
}

impl Answers {
    pub(crate) fn validate(&mut self) -> Result<(), Failure> {
        self.organization_name = self.organization_name.trim().to_owned();
        self.display_name = self.display_name.trim().to_owned();
        for (value, error) in [
            (&self.organization_name, "Invalid organization name"),
            (&self.display_name, "Invalid display name"),
        ] {
            if !(1..=80).contains(&value.chars().count()) || value.contains('\0') {
                return Err(Failure::field(error));
            }
        }
        if !(3..=254).contains(&self.email.len())
            || self.email.chars().filter(|&c| c == '@').count() != 1
            || self.email.chars().any(|c| c.is_whitespace() || c == '\0')
        {
            return Err(Failure::field("Invalid email"));
        }
        if self.password.chars().count() < 8 || self.password.len() > 1024 {
            return Err(Failure::field("Invalid password"));
        }
        if self.password != self.confirmation {
            return Err(Failure::field("Password confirmation does not match"));
        }
        let alphanumeric = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
        if !(1..=32).contains(&self.slug.len())
            || !self.slug.bytes().all(|b| alphanumeric(b) || b == b'-')
            || !self
                .slug
                .starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
            || !self
                .slug
                .ends_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        {
            return Err(Failure::field("Invalid organization slug"));
        }
        Ok(())
    }
}

pub(crate) async fn execute(pool: &PgPool, answers: Answers) -> Result<OrganizationId, Failure> {
    execute_with_hasher(pool, answers, auth::hash_password).await
}

pub(crate) async fn execute_with_hasher(
    pool: &PgPool,
    mut answers: Answers,
    hasher: fn(&str) -> Result<String, auth::PasswordHashError>,
) -> Result<OrganizationId, Failure> {
    answers.validate()?;
    let password_hash = tokio::task::spawn_blocking(move || hasher(&answers.password))
        .await
        .map_err(|_| Failure::generic())?
        .map_err(|_| Failure::generic())?;
    db::bootstrap_installation(
        pool,
        ActorId(Uuid::now_v7()),
        &answers.organization_name,
        &answers.slug,
        &answers.display_name,
        &answers.email,
        &password_hash,
    )
    .await
    .map_err(Failure::from)
}

pub(crate) async fn run() -> anyhow::Result<ExitCode> {
    let url = crate::config::database_url()?;
    let answers = tokio::task::spawn_blocking(prompt)
        .await
        .map_err(|_| anyhow::anyhow!("Bootstrap failed"))?;
    let result = match answers {
        Ok(answers) => {
            let slug = answers.slug.clone();
            let pool = db::connect(&url).await?;
            let result = execute(&pool, answers).await.map(|_| slug);
            pool.close().await;
            result
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(slug) => {
            output(&format!("{slug}\n")).map_err(|_| anyhow::anyhow!("Bootstrap output failed"))?;
            Ok(ExitCode::SUCCESS)
        }
        Err(failure) => {
            report(&failure);
            Ok(ExitCode::from(failure.code))
        }
    }
}

fn prompt() -> Result<Answers, Failure> {
    if !io::stdin().is_terminal() {
        return Err(Failure::field("Bootstrap requires stdin to be a terminal"));
    }
    Ok(Answers {
        organization_name: line("Organization name: ")?,
        slug: line("Organization slug: ")?,
        display_name: line("Display name: ")?,
        // Echoed, unlike the password: it is typed once, and a typo would lock the
        // installation's only owner out.
        email: line("Email: ")?,
        password: hidden("Password: ")?,
        confirmation: hidden("Confirm password: ")?,
    })
}

fn line(label: &str) -> Result<String, Failure> {
    output(label).map_err(|_| Failure::generic())?;
    let mut value = String::new();
    if io::stdin()
        .read_line(&mut value)
        .map_err(|_| Failure::generic())?
        == 0
    {
        return Err(Failure::generic());
    }
    if value.ends_with('\n') {
        value.pop();
        if value.ends_with('\r') {
            value.pop();
        }
    }
    Ok(value)
}

fn hidden(label: &str) -> Result<String, Failure> {
    output(label).map_err(|_| Failure::generic())?;
    rpassword::read_password().map_err(|_| Failure::generic())
}

#[expect(
    clippy::print_stdout,
    reason = "Bootstrap prompts and its successful slug are the terminal interface"
)]
fn output(text: &str) -> io::Result<()> {
    print!("{text}");
    io::stdout().flush()
}

#[expect(
    clippy::print_stderr,
    reason = "Bootstrap reports only fixed diagnostics, never submitted values"
)]
fn report(failure: &Failure) {
    eprintln!("{}", failure.message);
}
