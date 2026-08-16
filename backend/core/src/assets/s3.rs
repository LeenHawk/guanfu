//! S3 兼容对象存储(AWS S3、Cloudflare R2、MinIO)。
//!
//! 只用 GET/PUT/HEAD/DELETE 四个动作,自己做 SigV4 签名而不引入完整的
//! AWS SDK——这里的用法窄到不值得为它背一整套 SDK 的依赖与编译时间。

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use super::store::{store_error, AssetStore};
use crate::assets::ChunkHash;
use crate::CoreError;

/// 连接配置;凭证只在进程内存里,不写库不进日志。
#[derive(Clone)]
pub struct S3Config {
    /// 形如 `https://<account>.r2.cloudflarestorage.com`,不含桶名。
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    /// 对象键前缀,便于一个桶放多个环境。
    pub prefix: String,
}

impl std::fmt::Debug for S3Config {
    /// 手写 Debug:派生实现会把 secret 打进任何 `{:?}` 里。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Config")
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("prefix", &self.prefix)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct S3AssetStore {
    config: S3Config,
    http: reqwest::Client,
}

impl S3AssetStore {
    pub fn new(config: S3Config) -> Self {
        crate::llm::install_crypto_provider();
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    /// 从环境读取配置;缺少必需项返回 None(调用方回落到本地目录)。
    pub fn from_env() -> Option<S3Config> {
        let endpoint = std::env::var("GUANFU_S3_ENDPOINT").ok()?;
        let bucket = std::env::var("GUANFU_S3_BUCKET").ok()?;
        let access_key_id = std::env::var("GUANFU_S3_ACCESS_KEY_ID").ok()?;
        let secret_access_key = std::env::var("GUANFU_S3_SECRET_ACCESS_KEY").ok()?;
        Some(S3Config {
            endpoint: endpoint.trim_end_matches('/').to_owned(),
            bucket,
            // R2 只认 auto;AWS 要真实区域。
            region: std::env::var("GUANFU_S3_REGION").unwrap_or_else(|_| "auto".to_owned()),
            access_key_id,
            secret_access_key,
            prefix: std::env::var("GUANFU_S3_PREFIX").unwrap_or_else(|_| "chunks".to_owned()),
        })
    }

    /// 对象键:沿用本地实现的两位分桶,便于人工浏览。
    fn key_of(&self, hash: &ChunkHash) -> String {
        let (bucket, rest) = hash.0.split_at(2.min(hash.0.len()));
        let prefix = self.config.prefix.trim_matches('/');
        if prefix.is_empty() {
            format!("{bucket}/{rest}")
        } else {
            format!("{prefix}/{bucket}/{rest}")
        }
    }

    async fn send(
        &self,
        method: reqwest::Method,
        key: &str,
        body: Option<&[u8]>,
    ) -> Result<reqwest::Response, CoreError> {
        // path-style 寻址:R2 的自定义端点不支持 virtual-host 形式。
        let path = format!("/{}/{}", self.config.bucket, key);
        let url = format!("{}{}", self.config.endpoint, path);
        let host = self
            .config
            .endpoint
            .split("://")
            .nth(1)
            .ok_or_else(|| store_error("s3 endpoint must include a scheme"))?
            .to_owned();

        let payload = body.unwrap_or(&[]);
        let payload_hash = hex::encode(Sha256::digest(payload));
        let stamp = timestamp();
        let date = &stamp[..8];

        let canonical_headers =
            format!("host:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{stamp}\n");
        let signed_headers = "host;x-amz-content-sha256;x-amz-date";
        let canonical_request = format!(
            "{}\n{}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}",
            method.as_str(),
            uri_encode_path(&path),
        );

        let scope = format!("{date}/{}/s3/aws4_request", self.config.region);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{stamp}\n{scope}\n{}",
            hex::encode(Sha256::digest(canonical_request.as_bytes()))
        );

        let signature = hex::encode(sign_chain(
            &self.config.secret_access_key,
            date,
            &self.config.region,
            string_to_sign.as_bytes(),
        ));
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
            self.config.access_key_id
        );

        let mut request = self
            .http
            .request(method, url)
            .header("x-amz-content-sha256", payload_hash)
            .header("x-amz-date", stamp)
            .header(reqwest::header::AUTHORIZATION, authorization);
        if let Some(body) = body {
            request = request.body(body.to_vec());
        }
        request.send().await.map_err(store_error)
    }
}

#[async_trait::async_trait]
impl AssetStore for S3AssetStore {
    async fn put(&self, hash: &ChunkHash, bytes: &[u8]) -> Result<(), CoreError> {
        // 内容寻址意味着同键必同内容,重复写是浪费而不是冲突。
        if self.exists(hash).await {
            return Ok(());
        }
        let key = self.key_of(hash);
        let response = self.send(reqwest::Method::PUT, &key, Some(bytes)).await?;
        if !response.status().is_success() {
            return Err(failure("put", &key, response).await);
        }
        Ok(())
    }

    async fn get(&self, hash: &ChunkHash) -> Result<Vec<u8>, CoreError> {
        let key = self.key_of(hash);
        let response = self.send(reqwest::Method::GET, &key, None).await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::ChunkMissing(hash.0.clone()));
        }
        if !response.status().is_success() {
            return Err(failure("get", &key, response).await);
        }
        Ok(response.bytes().await.map_err(store_error)?.to_vec())
    }

    async fn exists(&self, hash: &ChunkHash) -> bool {
        match self
            .send(reqwest::Method::HEAD, &self.key_of(hash), None)
            .await
        {
            Ok(response) => response.status().is_success(),
            Err(_) => false,
        }
    }

    async fn delete(&self, hash: &ChunkHash) -> Result<(), CoreError> {
        let key = self.key_of(hash);
        let response = self.send(reqwest::Method::DELETE, &key, None).await?;
        // 删不存在的对象按成功处理,和本地实现一致。
        if response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        Err(failure("delete", &key, response).await)
    }
}

/// S3 的错误体是 XML,里面的 Code 才说明问题(签名不符 / 权限不足 /
/// 桶不存在);只报状态码会让排查无从下手。
async fn failure(action: &str, key: &str, response: reqwest::Response) -> CoreError {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let detail = body
        .split_once("<Code>")
        .and_then(|(_, rest)| rest.split_once("</Code>"))
        .map(|(code, _)| code.to_owned())
        .unwrap_or_else(|| body.chars().take(200).collect());
    store_error(format!("s3 {action} {key} failed with {status}: {detail}"))
}

type HmacSha256 = Hmac<Sha256>;

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn sign_chain(secret: &str, date: &str, region: &str, string_to_sign: &[u8]) -> Vec<u8> {
    let key = hmac(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let key = hmac(&key, region.as_bytes());
    let key = hmac(&key, b"s3");
    let key = hmac(&key, b"aws4_request");
    hmac(&key, string_to_sign)
}

/// `YYYYMMDDTHHMMSSZ`。
fn timestamp() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

/// SigV4 的 canonical URI:逐段编码,`/` 保留。
fn uri_encode_path(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            segment
                .bytes()
                .map(|byte| match byte {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                        (byte as char).to_string()
                    }
                    _ => format!("%{byte:02X}"),
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}
