use rand::{Rng, SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Clone, Deserialize, Serialize)]
#[serde(untagged, deny_unknown_fields)]
pub(crate) enum ServiceEvent {
    Message(String),
    Named { operation: String, message: String },
}
impl ServiceEvent {
    pub fn template(&self) -> &str {
        match self {
            Self::Message(message) | Self::Named { message, .. } => message,
        }
    }
    pub fn operation<'a>(&'a self, operations: &'a [String], single_event: bool) -> &'a str {
        if let Self::Named { operation, .. } = self {
            return operation;
        }
        let inferred = match self.template() {
            "User {user_id} registered successfully" => "user_registration",
            "Authentication failed for user {username}" => "user_authentication",
            "Profile updated for user {user_id}" => "profile_update",
            "Password changed for user {user_id}" => "password_change",
            "User {user_id} deleted" => "user_deletion",
            "Order {order_id} created for user {user_id}" => "order_creation",
            "Order {order_id} updated: status changed to {status}" => "order_update",
            "Order {order_id} cancelled by user {user_id}" => "order_cancellation",
            "Inventory check failed for product {product_id}" => "inventory_check",
            "Payment processed for order {order_id}: amount {amount}" => "payment_processing",
            "Product search: query='{query}', results={count}" => "product_search",
            "Inventory updated for product {product_id}: {old_stock} -> {new_stock}" => {
                "inventory_update"
            }
            "Price updated for product {product_id}: {old_price} -> {new_price}" => "price_update",
            "Recommendations generated for user {user_id}: {product_ids}" => {
                "product_recommendation"
            }
            "Payment authorized: transaction_id={transaction_id}, amount={amount}" => {
                "payment_authorization"
            }
            "Payment captured: transaction_id={transaction_id}" => "payment_capture",
            "Refund processed: transaction_id={transaction_id}, amount={amount}" => {
                "refund_processing"
            }
            "Fraud detected: transaction_id={transaction_id}, reason={reason}" => "fraud_detection",
            "Email sent to {email}: subject='{subject}'" => "email_sending",
            "SMS sent to {phone}: message='{message}'" => "sms_sending",
            "Push notification sent to user {user_id}: title='{title}'" => "push_notification",
            "Webhook delivered to {url}: payload_size={size}bytes" => "webhook_delivery",
            "Message published to queue '{queue}': routing_key='{routing_key}'" => {
                "message_published"
            }
            "Message consumed from queue '{queue}': consumer='{consumer_id}'" => "message_consumed",
            "Queue '{queue}' created with {message_count} messages" => "queue_created",
            "Consumer '{consumer_id}' connected to queue '{queue}'" => "consumer_connected",
            "Consumer '{consumer_id}' disconnected from queue '{queue}'" => "consumer_disconnected",
            _ => "event",
        };
        if operations.iter().any(|operation| operation == inferred) {
            inferred
        } else if operations.len() == 1 && single_event {
            &operations[0]
        } else {
            "event"
        }
    }
    pub fn validate(&self, operations: &[String]) -> io::Result<()> {
        if let Self::Named { operation, .. } = self
            && (operation.is_empty()
                || operation.chars().any(char::is_whitespace)
                || (!operations.is_empty() && !operations.contains(operation)))
        {
            return Err(io::Error::other("invalid or undeclared event operation"));
        }
        validate_event(self.template())
    }
}

pub(crate) fn validate_event(template: &str) -> io::Result<()> {
    if template.is_empty() || template.contains(['\n', '\r']) {
        return Err(io::Error::other("invalid microservice event template"));
    }
    render_event(template, "service", &[], &[], &mut StdRng::seed_from_u64(0))
        .map(|_| ())
        .map_err(io::Error::other)
}

pub(crate) fn render_event(
    template: &str,
    service: &str,
    databases: &[String],
    containers: &[String],
    rng: &mut StdRng,
) -> Result<String, String> {
    let user_id = rng.gen_range(1000..=99999).to_string();
    let order_id = format!("ORD-{}", rng.gen_range(100000..=999999));
    let product_id = rng.gen_range(1..=10000).to_string();
    let database = if databases.is_empty() {
        "app-db"
    } else {
        &databases[rng.gen_range(0..databases.len())]
    };
    let container = if containers.is_empty() {
        "app-1".to_owned()
    } else {
        let name = &containers[rng.gen_range(0..containers.len())];
        name.replace("{instance}", &rng.gen_range(1..=99).to_string())
    };
    let values = [
        ("service_name", service.to_owned()),
        ("user_id", user_id),
        ("username", "user".to_owned()),
        ("order_id", order_id),
        ("product_id", product_id),
        ("status", "completed".to_owned()),
        ("amount", format!("{:.2}", rng.gen_range(1.0..1000.0))),
        ("query", "SELECT * FROM items".to_owned()),
        ("count", rng.gen_range(1..=50).to_string()),
        ("old_stock", "20".to_owned()),
        ("new_stock", "19".to_owned()),
        ("old_price", "20.00".to_owned()),
        ("new_price", "19.00".to_owned()),
        ("product_ids", "1,2,3".to_owned()),
        ("transaction_id", format!("txn-{:08x}", rng.r#gen::<u32>())),
        ("reason", "timeout".to_owned()),
        ("module", "app".to_owned()),
        ("exception_type", "RuntimeError".to_owned()),
        ("usage", "1024".to_owned()),
        ("email", "user@example.test".to_owned()),
        ("phone", "+15550100000".to_owned()),
        ("subject", "Notification".to_owned()),
        ("message", "Delivery update".to_owned()),
        ("title", "Order update".to_owned()),
        ("url", "https://example.test/webhook".to_owned()),
        ("size", "512".to_owned()),
        ("queue", "orders".to_owned()),
        ("queue_name", "orders".to_owned()),
        ("routing_key", "orders.created".to_owned()),
        ("consumer_id", "consumer-1".to_owned()),
        ("message_count", "10".to_owned()),
        ("db_name", database.to_owned()),
        ("duration", rng.gen_range(1..=1000).to_string()),
        ("container_name", container),
        ("instance", "1".to_owned()),
        ("memory_usage", "256".to_owned()),
        ("cpu_usage", "42".to_owned()),
    ];
    crate::patterns::render(template, &values)
}
