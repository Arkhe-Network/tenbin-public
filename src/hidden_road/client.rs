use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use chrono::Utc;
use reqwest::header::{CONTENT_TYPE, HeaderValue};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tracing::{debug, instrument, warn};

use crate::{
    hidden_road::types::{
        AccountActivityAccountsResponse, CalculationsResponse, OtcAssetsResponse,
        OtcBalancesResponse, OtcCounterpartiesResponse, OtcInstrumentsResponse, OtcMarketsResponse,
        Paginated, PermittedAsset, RiskMetrics, RiskMetricsDetails, Route28FundingRatesResponse,
        Route28PositionsResponse, Route28TradesResponse,
    },
    types::hidden_road::{
        AssetConversion, AssetConversionRequest, AssetTransfer, AssetTransferRequest, Deposit,
        DepositRequest, DigitalAssetsResponse, OtcReportTradeRequest, OtcReportTradeResponse,
        OtcTradesResponse, PaymentAccount, PositionsOrBalancesResponse, TradesResponse,
        TransfersResponse, Withdrawal, WithdrawalRequest,
    },
};

/// Expiry refresh buffer in seconds
pub const EXPIRY_SKEW_SECS: i64 = 30;
/// API version
pub const API_VERSION: &str = "v0";
/// API audience
pub const API_AUDIENCE: &str = "https://api.hiddenroad.com/v0/";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HiddenRoadError {
    /// Authentication issues
    Auth(String),
    /// HTTP failures
    Http { endpoint: String, msg: String },
    /// JSON deserialisation errors
    Deserialize { endpoint: String, msg: String },
}

impl std::fmt::Display for HiddenRoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HiddenRoadError::Auth(msg) => write!(f, "auth error: {msg}"),
            HiddenRoadError::Http { endpoint, msg } => {
                write!(f, "http error on {endpoint}: {msg}")
            }
            HiddenRoadError::Deserialize { endpoint, msg } => {
                write!(f, "deserialize error on {endpoint}: {msg}")
            }
        }
    }
}

impl std::error::Error for HiddenRoadError {}

#[derive(Debug, Clone, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    expires_in: i64,
}

#[derive(Debug, Clone)]
struct Token {
    value: String,
    expires: i64,
}

/// Hidden Road API client
///
/// Supports:
/// - ATM               `atm/*`
/// - AccountActivity   `accountactivity/*`
/// - Route28           `route28/*`
/// - Risk Metrics      `metrics/*`
/// - OTC               `otc/*`
#[derive(Clone)]
pub struct HiddenRoadClient {
    /// Client ID
    client_id: String,
    /// Client secret
    client_secret: String,
    /// Base URI
    base_uri: String,
    /// Auth base URI
    auth_base_uri: String,
    /// HTTP client
    http: reqwest::Client,
    /// Authentication token
    token: Option<Token>,
}

impl HiddenRoadClient {
    pub fn new(client_id: &str, client_secret: &str, base_uri: &str, auth_base_uri: &str) -> Self {
        // Per-request timeout. Without this, a slow / hung HR endpoint blocks
        // the caller indefinitely with no log output — that's what caused the
        // 3.5-minute silent gap in the May-15 incident where all five HR
        // sync tasks stalled together on a token refresh. A 30s timeout
        // turns the next such event into loud HTTP errors instead.
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .local_address(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
            .no_proxy()
            .build()
            .expect("failed to build reqwest client");

        Self {
            client_id: client_id.to_owned(),
            client_secret: client_secret.to_owned(),
            base_uri: base_uri.to_owned(),
            auth_base_uri: auth_base_uri.to_owned(),
            http,
            token: None,
        }
    }

    fn api_url(&self, path: &str) -> String {
        format!("{}/{}/{}", self.base_uri, API_VERSION, path)
    }

    fn token_expired(&self) -> bool {
        let now = Utc::now().timestamp();
        match self.token.as_ref() {
            Some(t) => t.expires.saturating_sub(EXPIRY_SKEW_SECS) <= now,
            None => true,
        }
    }

    fn ensure_token(&self) -> Result<&Token, HiddenRoadError> {
        self.token.as_ref().ok_or_else(|| {
            HiddenRoadError::Auth("bearer token missing; call refresh_token first".into())
        })
    }

    #[instrument(skip(self))]
    pub async fn refresh_token(&mut self) -> Result<(), HiddenRoadError> {
        if !self.token_expired() {
            return Ok(());
        }

        #[derive(Serialize)]
        struct AuthRequest<'a> {
            client_id: &'a str,
            client_secret: &'a str,
            grant_type: &'a str,
            audience: &'a str,
        }

        let body = AuthRequest {
            client_id: &self.client_id,
            client_secret: &self.client_secret,
            grant_type: "client_credentials",
            audience: API_AUDIENCE,
        };

        let resp = self
            .http
            .post(format!("{}/oauth/token", self.auth_base_uri))
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .json(&body)
            .send()
            .await
            .map_err(|e| HiddenRoadError::Auth(format!("token request failed: {e:?}")))?;

        if !resp.status().is_success() {
            let code = resp.status();
            let body = resp.text().await.unwrap_or_default();
            warn!(code = %code, body = %body, "Hidden Road auth endpoint returned");
            return Err(HiddenRoadError::Auth(format!(
                "auth endpoint returned {code}"
            )));
        }

        let now = Utc::now().timestamp();
        let tr: OAuthTokenResponse = resp
            .json()
            .await
            .map_err(|e| HiddenRoadError::Auth(format!("token response parse failed: {e}")))?;

        self.token = Some(Token {
            value: tr.access_token,
            expires: now + tr.expires_in,
        });

        Ok(())
    }

    async fn get_json<T: DeserializeOwned>(
        &mut self,
        endpoint: &'static str,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, HiddenRoadError> {
        self.refresh_token().await?;
        let token = self.ensure_token()?;
        let url = self.api_url(path);

        let auth_header = format!("Bearer {}", token.value.trim());

        let resp = self
            .http
            .get(&url)
            .header("Authorization", auth_header)
            .query(query)
            .send()
            .await
            .map_err(|e| HiddenRoadError::Http {
                endpoint: endpoint.to_owned(),
                msg: format!("request failed: {e}"),
            })?;

        if !resp.status().is_success() {
            let code = resp.status();
            let body = resp.text().await.unwrap_or_default();
            warn!(code = %code, body = %body, "Hidden Road GET returned error");
            return Err(HiddenRoadError::Http {
                endpoint: endpoint.to_owned(),
                msg: format!("endpoint returned {code}"),
            });
        }

        let body = resp.text().await.unwrap_or_default();
        debug!("{}", body);

        serde_json::from_str::<T>(&body).map_err(|e| HiddenRoadError::Deserialize {
            endpoint: endpoint.to_owned(),
            msg: format!("response deserialization failed: {e}"),
        })
    }

    async fn post_json<R: Serialize, B: DeserializeOwned>(
        &mut self,
        endpoint: &'static str,
        path: &str,
        body: &R,
    ) -> Result<B, HiddenRoadError> {
        self.refresh_token().await?;
        let token = self.ensure_token()?;
        let url = self.api_url(path);

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token.value)
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .json(body)
            .send()
            .await
            .map_err(|e| HiddenRoadError::Http {
                endpoint: endpoint.to_owned(),
                msg: format!("request failed: {e}"),
            })?;

        if !resp.status().is_success() {
            let code = resp.status();
            let body_str = resp.text().await.unwrap_or_default();
            warn!(code = %code, body = %body_str, "Hidden Road POST returned error");
            return Err(HiddenRoadError::Http {
                endpoint: endpoint.to_owned(),
                msg: format!("endpoint returned {code}"),
            });
        }

        let body_str = resp.text().await.unwrap_or_default();
        debug!("{}", body_str);

        serde_json::from_str::<B>(&body_str).map_err(|e| HiddenRoadError::Deserialize {
            endpoint: endpoint.to_owned(),
            msg: format!("response deserialization failed: {e}"),
        })
    }

    #[instrument(skip(self))]
    pub async fn atm_create_transfer(
        &mut self,
        request: AssetTransferRequest,
    ) -> Result<AssetTransfer, HiddenRoadError> {
        self.post_json("atm_create_transfer", "atm/transfer", &request)
            .await
    }

    #[instrument(skip(self))]
    pub async fn atm_get_transfers(
        &mut self,
        transfer_id: Option<&str>,
        offset: Option<u64>,
        sort_by: Option<&str>,
    ) -> Result<Paginated<AssetTransfer>, HiddenRoadError> {
        let mut q = Vec::new();
        if let Some(v) = transfer_id {
            q.push(("transfer_id", v.to_owned()));
        }
        if let Some(v) = offset {
            q.push(("offset", v.to_string()));
        }
        if let Some(v) = sort_by {
            q.push(("sort_by", v.to_owned()));
        }

        self.get_json("atm_get_transfers", "atm/transfer", &q).await
    }

    #[instrument(skip(self))]
    pub async fn atm_create_deposit(
        &mut self,
        request: DepositRequest,
    ) -> Result<Deposit, HiddenRoadError> {
        self.post_json("atm_create_deposit", "atm/deposits", &request)
            .await
    }

    #[instrument(skip(self))]
    pub async fn atm_get_deposits(
        &mut self,
        deposit_id: Option<&str>,
        offset: Option<u64>,
        sort_by: Option<&str>,
    ) -> Result<Paginated<Deposit>, HiddenRoadError> {
        let mut q = Vec::new();
        if let Some(v) = deposit_id {
            q.push(("deposit_id", v.to_owned()));
        }
        if let Some(v) = offset {
            q.push(("offset", v.to_string()));
        }
        if let Some(v) = sort_by {
            q.push(("sort_by", v.to_owned()));
        }

        self.get_json("atm_get_deposits", "atm/deposits", &q).await
    }

    #[instrument(skip(self))]
    pub async fn atm_create_withdrawal(
        &mut self,
        request: WithdrawalRequest,
    ) -> Result<Withdrawal, HiddenRoadError> {
        self.post_json("atm_create_withdrawal", "atm/withdrawals", &request)
            .await
    }

    #[instrument(skip(self))]
    pub async fn atm_get_withdrawals(
        &mut self,
        withdrawal_id: Option<&str>,
        offset: Option<u64>,
        sort_by: Option<&str>,
    ) -> Result<Paginated<Withdrawal>, HiddenRoadError> {
        let mut q = Vec::new();
        if let Some(v) = withdrawal_id {
            q.push(("withdrawal_id", v.to_owned()));
        }
        if let Some(v) = offset {
            q.push(("offset", v.to_string()));
        }
        if let Some(v) = sort_by {
            q.push(("sort_by", v.to_owned()));
        }

        self.get_json("atm_get_withdrawals", "atm/withdrawals", &q)
            .await
    }

    #[instrument(skip(self))]
    pub async fn atm_create_asset_conversion(
        &mut self,
        request: AssetConversionRequest,
    ) -> Result<AssetConversion, HiddenRoadError> {
        self.post_json(
            "atm_create_asset_conversion",
            "atm/asset-conversions",
            &request,
        )
        .await
    }

    #[instrument(skip(self))]
    pub async fn atm_get_asset_conversions(
        &mut self,
        asset_conversion_id: Option<&str>,
    ) -> Result<Paginated<AssetConversion>, HiddenRoadError> {
        let mut q = Vec::new();
        if let Some(v) = asset_conversion_id {
            q.push(("asset_conversion_id", v.to_owned()));
        }

        self.get_json("atm_get_asset_conversions", "atm/asset-conversions", &q)
            .await
    }

    #[instrument(skip(self))]
    pub async fn atm_get_payment_accounts(
        &mut self,
        counterparty: &str,
        payment_account_id: Option<&str>,
        status: Option<&str>,
    ) -> Result<Paginated<PaymentAccount>, HiddenRoadError> {
        let mut q = Vec::new();
        q.push(("counterparty", counterparty.to_owned()));
        if let Some(v) = payment_account_id {
            q.push(("payment_account_id", v.to_owned()));
        }
        if let Some(v) = status {
            q.push(("status", v.to_owned()));
        }

        self.get_json("atm_get_payment_accounts", "atm/payment_accounts", &q)
            .await
    }

    #[instrument(skip(self))]
    pub async fn atm_get_digital_assets(&mut self) -> Result<Vec<String>, HiddenRoadError> {
        let resp: DigitalAssetsResponse = self
            .get_json("atm_get_digital_assets", "atm/assets/digital", &[])
            .await?;
        Ok(resp.assets)
    }

    #[instrument(skip(self))]
    pub async fn route28_get_trades(
        &mut self,
        party_to: Option<bool>,
        counterparty_to: Option<bool>,
        day: Option<&str>,
        cursor_val: Option<&str>,
        size: Option<u32>,
    ) -> Result<Route28TradesResponse, HiddenRoadError> {
        let mut q = Vec::new();
        if let Some(v) = party_to {
            q.push(("party_to", v.to_string()));
        }
        if let Some(v) = counterparty_to {
            q.push(("counterparty_to", v.to_string()));
        }
        if let Some(v) = day {
            q.push(("day", v.to_owned()));
        }
        if let Some(v) = cursor_val {
            q.push(("cursor_val", v.to_owned()));
        }
        if let Some(v) = size {
            q.push(("size", v.to_string()));
        }

        self.get_json("route28_get_trades", "route28/trades", &q)
            .await
    }

    #[instrument(skip(self))]
    pub async fn route28_get_positions(
        &mut self,
        end_event_timestamp_exclusive: Option<&str>,
    ) -> Result<Route28PositionsResponse, HiddenRoadError> {
        let mut q = Vec::new();
        if let Some(v) = end_event_timestamp_exclusive {
            q.push(("end_event_timestamp_exclusive", v.to_owned()));
        }

        self.get_json("route28_get_positions", "route28/positions", &q)
            .await
    }

    #[instrument(skip(self))]
    pub async fn route28_get_funding_rates(
        &mut self,
    ) -> Result<Route28FundingRatesResponse, HiddenRoadError> {
        self.get_json("route28_get_funding_rates", "route28/funding-rates", &[])
            .await
    }

    #[instrument(skip(self))]
    pub async fn account_activity_ping(&mut self) -> Result<(), HiddenRoadError> {
        let _: serde_json::Value = self
            .get_json("accountactivity_ping", "accountactivity/ping", &[])
            .await?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn account_activity_list_accounts(
        &mut self,
    ) -> Result<AccountActivityAccountsResponse, HiddenRoadError> {
        self.get_json(
            "accountactivity_list_accounts",
            "accountactivity/accounts",
            &[],
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    #[instrument(skip(self))]
    pub async fn account_activity_get_trades(
        &mut self,
        venue: Option<&str>,
        account: Option<&str>,
        start_record_timestamp_inclusive: Option<&str>,
        end_record_timestamp_exclusive: Option<&str>,
        start_event_timestamp_inclusive: Option<&str>,
        end_event_timestamp_exclusive: Option<&str>,
        page_size: Option<u32>,
        start_record_event_id: Option<&str>,
        start_record_correction_version: Option<i32>,
    ) -> Result<TradesResponse, HiddenRoadError> {
        let mut q = Vec::new();
        if let Some(v) = venue {
            q.push(("venue", v.to_owned()));
        }
        if let Some(v) = account {
            q.push(("account", v.to_owned()));
        }
        if let Some(v) = start_record_timestamp_inclusive {
            q.push(("start_record_timestamp_inclusive", v.to_owned()));
        }
        if let Some(v) = end_record_timestamp_exclusive {
            q.push(("end_record_timestamp_exclusive", v.to_owned()));
        }
        if let Some(v) = start_event_timestamp_inclusive {
            q.push(("start_event_timestamp_inclusive", v.to_owned()));
        }
        if let Some(v) = end_event_timestamp_exclusive {
            q.push(("end_event_timestamp_exclusive", v.to_owned()));
        }
        if let Some(v) = page_size {
            q.push(("page_size", v.to_string()));
        }
        if let Some(v) = start_record_event_id {
            q.push(("start_record_event_id", v.to_owned()));
        }
        if let Some(v) = start_record_correction_version {
            q.push(("start_record_correction_version", v.to_string()));
        }

        self.get_json("accountactivity_get_trades", "accountactivity/trades", &q)
            .await
    }

    #[instrument(skip(self))]
    pub async fn account_activity_get_positions_snapshots(
        &mut self,
        venue: &str,
        account: Option<&str>,
        page_size: Option<u32>,
        end_event_timestamp_exclusive: Option<&str>,
    ) -> Result<PositionsOrBalancesResponse, HiddenRoadError> {
        let mut q = Vec::new();
        q.push(("venue", venue.to_owned()));
        if let Some(v) = account {
            q.push(("account", v.to_owned()));
        }
        if let Some(v) = page_size {
            q.push(("page_size", v.to_string()));
        }
        if let Some(v) = end_event_timestamp_exclusive {
            q.push(("end_event_timestamp_exclusive", v.to_owned()));
        }

        self.get_json(
            "accountactivity_get_positions_snapshots",
            "accountactivity/positions-snapshots",
            &q,
        )
        .await
    }

    #[instrument(skip(self))]
    pub async fn account_activity_get_balances_snapshots(
        &mut self,
        venue: &str,
        account: Option<&str>,
        page_size: Option<u32>,
        end_event_timestamp_exclusive: Option<&str>,
    ) -> Result<PositionsOrBalancesResponse, HiddenRoadError> {
        let mut q = Vec::new();
        q.push(("venue", venue.to_owned()));
        if let Some(v) = account {
            q.push(("account", v.to_owned()));
        }
        if let Some(v) = page_size {
            q.push(("page_size", v.to_string()));
        }
        if let Some(v) = end_event_timestamp_exclusive {
            q.push(("end_event_timestamp_exclusive", v.to_owned()));
        }

        self.get_json(
            "accountactivity_get_balances_snapshots",
            "accountactivity/balances-snapshots",
            &q,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    #[instrument(skip(self))]
    pub async fn account_activity_get_deposits(
        &mut self,
        venue: Option<&str>,
        account: Option<&str>,
        start_record_timestamp_inclusive: Option<&str>,
        end_record_timestamp_exclusive: Option<&str>,
        start_event_timestamp_inclusive: Option<&str>,
        end_event_timestamp_exclusive: Option<&str>,
        page_size: Option<u32>,
        start_record_event_id: Option<&str>,
        start_record_correction_version: Option<i32>,
    ) -> Result<TransfersResponse, HiddenRoadError> {
        let mut q = Vec::new();
        if let Some(v) = venue {
            q.push(("venue", v.to_owned()));
        }
        if let Some(v) = account {
            q.push(("account", v.to_owned()));
        }
        if let Some(v) = start_record_timestamp_inclusive {
            q.push(("start_record_timestamp_inclusive", v.to_owned()));
        }
        if let Some(v) = end_record_timestamp_exclusive {
            q.push(("end_record_timestamp_exclusive", v.to_owned()));
        }
        if let Some(v) = start_event_timestamp_inclusive {
            q.push(("start_event_timestamp_inclusive", v.to_owned()));
        }
        if let Some(v) = end_event_timestamp_exclusive {
            q.push(("end_event_timestamp_exclusive", v.to_owned()));
        }
        if let Some(v) = page_size {
            q.push(("page_size", v.to_string()));
        }
        if let Some(v) = start_record_event_id {
            q.push(("start_record_event_id", v.to_owned()));
        }
        if let Some(v) = start_record_correction_version {
            q.push(("start_record_correction_version", v.to_string()));
        }

        self.get_json(
            "accountactivity_get_deposits",
            "accountactivity/transfers/deposits",
            &q,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    #[instrument(skip(self))]
    pub async fn account_activity_get_withdrawals(
        &mut self,
        venue: Option<&str>,
        account: Option<&str>,
        start_record_timestamp_inclusive: Option<&str>,
        end_record_timestamp_exclusive: Option<&str>,
        start_event_timestamp_inclusive: Option<&str>,
        end_event_timestamp_exclusive: Option<&str>,
        page_size: Option<u32>,
        start_record_event_id: Option<&str>,
        start_record_correction_version: Option<i32>,
    ) -> Result<TransfersResponse, HiddenRoadError> {
        let mut q = Vec::new();
        if let Some(v) = venue {
            q.push(("venue", v.to_owned()));
        }
        if let Some(v) = account {
            q.push(("account", v.to_owned()));
        }
        if let Some(v) = start_record_timestamp_inclusive {
            q.push(("start_record_timestamp_inclusive", v.to_owned()));
        }
        if let Some(v) = end_record_timestamp_exclusive {
            q.push(("end_record_timestamp_exclusive", v.to_owned()));
        }
        if let Some(v) = start_event_timestamp_inclusive {
            q.push(("start_event_timestamp_inclusive", v.to_owned()));
        }
        if let Some(v) = end_event_timestamp_exclusive {
            q.push(("end_event_timestamp_exclusive", v.to_owned()));
        }
        if let Some(v) = page_size {
            q.push(("page_size", v.to_string()));
        }
        if let Some(v) = start_record_event_id {
            q.push(("start_record_event_id", v.to_owned()));
        }
        if let Some(v) = start_record_correction_version {
            q.push(("start_record_correction_version", v.to_string()));
        }

        self.get_json(
            "accountactivity_get_withdrawals",
            "accountactivity/transfers/withdrawals",
            &q,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    #[instrument(skip(self))]
    pub async fn account_activity_get_credits(
        &mut self,
        venue: Option<&str>,
        account: Option<&str>,
        start_record_timestamp_inclusive: Option<&str>,
        end_record_timestamp_exclusive: Option<&str>,
        start_event_timestamp_inclusive: Option<&str>,
        end_event_timestamp_exclusive: Option<&str>,
        page_size: Option<u32>,
        start_record_event_id: Option<&str>,
        start_record_correction_version: Option<i32>,
    ) -> Result<TransfersResponse, HiddenRoadError> {
        let mut q = Vec::new();
        if let Some(v) = venue {
            q.push(("venue", v.to_owned()));
        }
        if let Some(v) = account {
            q.push(("account", v.to_owned()));
        }
        if let Some(v) = start_record_timestamp_inclusive {
            q.push(("start_record_timestamp_inclusive", v.to_owned()));
        }
        if let Some(v) = end_record_timestamp_exclusive {
            q.push(("end_record_timestamp_exclusive", v.to_owned()));
        }
        if let Some(v) = start_event_timestamp_inclusive {
            q.push(("start_event_timestamp_inclusive", v.to_owned()));
        }
        if let Some(v) = end_event_timestamp_exclusive {
            q.push(("end_event_timestamp_exclusive", v.to_owned()));
        }
        if let Some(v) = page_size {
            q.push(("page_size", v.to_string()));
        }
        if let Some(v) = start_record_event_id {
            q.push(("start_record_event_id", v.to_owned()));
        }
        if let Some(v) = start_record_correction_version {
            q.push(("start_record_correction_version", v.to_string()));
        }

        self.get_json(
            "accountactivity_get_credits",
            "accountactivity/adjustments/credits",
            &q,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    #[instrument(skip(self))]
    pub async fn account_activity_get_debits(
        &mut self,
        venue: Option<&str>,
        account: Option<&str>,
        start_record_timestamp_inclusive: Option<&str>,
        end_record_timestamp_exclusive: Option<&str>,
        start_event_timestamp_inclusive: Option<&str>,
        end_event_timestamp_exclusive: Option<&str>,
        page_size: Option<u32>,
        start_record_event_id: Option<&str>,
        start_record_correction_version: Option<i32>,
    ) -> Result<TransfersResponse, HiddenRoadError> {
        let mut q = Vec::new();
        if let Some(v) = venue {
            q.push(("venue", v.to_owned()));
        }
        if let Some(v) = account {
            q.push(("account", v.to_owned()));
        }
        if let Some(v) = start_record_timestamp_inclusive {
            q.push(("start_record_timestamp_inclusive", v.to_owned()));
        }
        if let Some(v) = end_record_timestamp_exclusive {
            q.push(("end_record_timestamp_exclusive", v.to_owned()));
        }
        if let Some(v) = start_event_timestamp_inclusive {
            q.push(("start_event_timestamp_inclusive", v.to_owned()));
        }
        if let Some(v) = end_event_timestamp_exclusive {
            q.push(("end_event_timestamp_exclusive", v.to_owned()));
        }
        if let Some(v) = page_size {
            q.push(("page_size", v.to_string()));
        }
        if let Some(v) = start_record_event_id {
            q.push(("start_record_event_id", v.to_owned()));
        }
        if let Some(v) = start_record_correction_version {
            q.push(("start_record_correction_version", v.to_string()));
        }

        self.get_json(
            "accountactivity_get_debits",
            "accountactivity/adjustments/debits",
            &q,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    #[instrument(skip(self))]
    pub async fn account_activity_get_cip(
        &mut self,
        venue: Option<&str>,
        account: Option<&str>,
        start_record_timestamp_inclusive: Option<&str>,
        end_record_timestamp_exclusive: Option<&str>,
        start_event_timestamp_inclusive: Option<&str>,
        end_event_timestamp_exclusive: Option<&str>,
        page_size: Option<u32>,
        start_record_event_id: Option<&str>,
        start_record_correction_version: Option<i32>,
    ) -> Result<CalculationsResponse, HiddenRoadError> {
        let mut q = Vec::new();
        if let Some(v) = venue {
            q.push(("venue", v.to_owned()));
        }
        if let Some(v) = account {
            q.push(("account", v.to_owned()));
        }
        if let Some(v) = start_record_timestamp_inclusive {
            q.push(("start_record_timestamp_inclusive", v.to_owned()));
        }
        if let Some(v) = end_record_timestamp_exclusive {
            q.push(("end_record_timestamp_exclusive", v.to_owned()));
        }
        if let Some(v) = start_event_timestamp_inclusive {
            q.push(("start_event_timestamp_inclusive", v.to_owned()));
        }
        if let Some(v) = end_event_timestamp_exclusive {
            q.push(("end_event_timestamp_exclusive", v.to_owned()));
        }
        if let Some(v) = page_size {
            q.push(("page_size", v.to_string()));
        }
        if let Some(v) = start_record_event_id {
            q.push(("start_record_event_id", v.to_owned()));
        }
        if let Some(v) = start_record_correction_version {
            q.push(("start_record_correction_version", v.to_string()));
        }

        self.get_json(
            "accountactivity_get_cip",
            "accountactivity/calculations/v0/cip",
            &q,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    #[instrument(skip(self))]
    pub async fn account_activity_get_ers(
        &mut self,
        venue: Option<&str>,
        account: Option<&str>,
        start_record_timestamp_inclusive: Option<&str>,
        end_record_timestamp_exclusive: Option<&str>,
        start_event_timestamp_inclusive: Option<&str>,
        end_event_timestamp_exclusive: Option<&str>,
        page_size: Option<u32>,
        start_record_event_id: Option<&str>,
        start_record_correction_version: Option<i32>,
    ) -> Result<CalculationsResponse, HiddenRoadError> {
        let mut q = Vec::new();
        if let Some(v) = venue {
            q.push(("venue", v.to_owned()));
        }
        if let Some(v) = account {
            q.push(("account", v.to_owned()));
        }
        if let Some(v) = start_record_timestamp_inclusive {
            q.push(("start_record_timestamp_inclusive", v.to_owned()));
        }
        if let Some(v) = end_record_timestamp_exclusive {
            q.push(("end_record_timestamp_exclusive", v.to_owned()));
        }
        if let Some(v) = start_event_timestamp_inclusive {
            q.push(("start_event_timestamp_inclusive", v.to_owned()));
        }
        if let Some(v) = end_event_timestamp_exclusive {
            q.push(("end_event_timestamp_exclusive", v.to_owned()));
        }
        if let Some(v) = page_size {
            q.push(("page_size", v.to_string()));
        }
        if let Some(v) = start_record_event_id {
            q.push(("start_record_event_id", v.to_owned()));
        }
        if let Some(v) = start_record_correction_version {
            q.push(("start_record_correction_version", v.to_string()));
        }

        self.get_json(
            "accountactivity_get_ers",
            "accountactivity/calculations/v0/ers",
            &q,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    #[instrument(skip(self))]
    pub async fn account_activity_get_financing(
        &mut self,
        venue: Option<&str>,
        account: Option<&str>,
        start_record_timestamp_inclusive: Option<&str>,
        end_record_timestamp_exclusive: Option<&str>,
        start_event_timestamp_inclusive: Option<&str>,
        end_event_timestamp_exclusive: Option<&str>,
        page_size: Option<u32>,
        start_record_event_id: Option<&str>,
        start_record_correction_version: Option<i32>,
    ) -> Result<CalculationsResponse, HiddenRoadError> {
        let mut q = Vec::new();
        if let Some(v) = venue {
            q.push(("venue", v.to_owned()));
        }
        if let Some(v) = account {
            q.push(("account", v.to_owned()));
        }
        if let Some(v) = start_record_timestamp_inclusive {
            q.push(("start_record_timestamp_inclusive", v.to_owned()));
        }
        if let Some(v) = end_record_timestamp_exclusive {
            q.push(("end_record_timestamp_exclusive", v.to_owned()));
        }
        if let Some(v) = start_event_timestamp_inclusive {
            q.push(("start_event_timestamp_inclusive", v.to_owned()));
        }
        if let Some(v) = end_event_timestamp_exclusive {
            q.push(("end_event_timestamp_exclusive", v.to_owned()));
        }
        if let Some(v) = page_size {
            q.push(("page_size", v.to_string()));
        }
        if let Some(v) = start_record_event_id {
            q.push(("start_record_event_id", v.to_owned()));
        }
        if let Some(v) = start_record_correction_version {
            q.push(("start_record_correction_version", v.to_string()));
        }

        self.get_json(
            "accountactivity_get_financing",
            "accountactivity/calculations/v0/financing",
            &q,
        )
        .await
    }

    #[instrument(skip(self))]
    pub async fn metrics_get_risk(&mut self) -> Result<RiskMetrics, HiddenRoadError> {
        self.get_json("metrics_get_risk", "metrics/risk", &[]).await
    }

    #[instrument(skip(self))]
    pub async fn metrics_get_risk_details(
        &mut self,
    ) -> Result<RiskMetricsDetails, HiddenRoadError> {
        self.get_json("metrics_get_risk_details", "metrics/risk/details", &[])
            .await
    }

    #[instrument(skip(self))]
    pub async fn metrics_get_permitted_assets(
        &mut self,
    ) -> Result<Vec<PermittedAsset>, HiddenRoadError> {
        self.get_json(
            "metrics_get_permitted_assets",
            "metrics/permitted-assets",
            &[],
        )
        .await
    }

    #[instrument(skip(self))]
    pub async fn otc_report_trade(
        &mut self,
        request: OtcReportTradeRequest,
    ) -> Result<OtcReportTradeResponse, HiddenRoadError> {
        self.post_json("otc_report_trade", "otc/trades", &request)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    #[instrument(skip(self))]
    pub async fn otc_get_trades(
        &mut self,
        matched: Option<bool>,
        settled: Option<bool>,
        reporter_of: Option<bool>,
        party_to: Option<bool>,
        counterparty_to: Option<bool>,
        day: Option<&str>,
        cursor_val: Option<&str>,
        size: Option<u32>,
    ) -> Result<OtcTradesResponse, HiddenRoadError> {
        let mut q = Vec::new();
        if let Some(v) = matched {
            q.push(("matched", v.to_string()));
        }
        if let Some(v) = settled {
            q.push(("settled", v.to_string()));
        }
        if let Some(v) = reporter_of {
            q.push(("reporter_of", v.to_string()));
        }
        if let Some(v) = party_to {
            q.push(("party_to", v.to_string()));
        }
        if let Some(v) = counterparty_to {
            q.push(("counterparty_to", v.to_string()));
        }
        if let Some(v) = day {
            q.push(("day", v.to_owned()));
        }
        if let Some(v) = cursor_val {
            q.push(("cursor_val", v.to_owned()));
        }
        if let Some(v) = size {
            q.push(("size", v.to_string()));
        }

        self.get_json("otc_get_trades", "otc/trades", &q).await
    }

    #[instrument(skip(self))]
    pub async fn otc_get_balances(&mut self) -> Result<OtcBalancesResponse, HiddenRoadError> {
        self.get_json("otc_get_balances", "otc/balances", &[]).await
    }

    #[instrument(skip(self))]
    pub async fn otc_get_counterparties(
        &mut self,
    ) -> Result<OtcCounterpartiesResponse, HiddenRoadError> {
        self.get_json(
            "otc_get_counterparties",
            "otc/relationships/counterparts",
            &[],
        )
        .await
    }

    #[instrument(skip(self))]
    pub async fn otc_get_markets(&mut self) -> Result<OtcMarketsResponse, HiddenRoadError> {
        self.get_json("otc_get_markets", "otc/markets", &[]).await
    }

    #[instrument(skip(self))]
    pub async fn otc_get_instruments(
        &mut self,
        cursor_val: Option<&str>,
        size: Option<u32>,
    ) -> Result<OtcInstrumentsResponse, HiddenRoadError> {
        let mut q = Vec::new();
        if let Some(v) = cursor_val {
            q.push(("cursor_val", v.to_owned()));
        }
        if let Some(v) = size {
            q.push(("size", v.to_string()));
        }

        self.get_json("otc_get_instruments", "otc/instruments", &q)
            .await
    }

    #[instrument(skip(self))]
    pub async fn otc_get_assets(
        &mut self,
        cursor_val: Option<&str>,
        size: Option<u32>,
    ) -> Result<OtcAssetsResponse, HiddenRoadError> {
        let mut q = Vec::new();
        if let Some(v) = cursor_val {
            q.push(("cursor_val", v.to_owned()));
        }
        if let Some(v) = size {
            q.push(("size", v.to_string()));
        }

        self.get_json("otc_get_assets", "otc/assets", &q).await
    }
}
