//! OpenAI LLM provider implementation

use crate::command::CommandArgs;
use crate::llm::{CommandMetadata, CommandResolution, LlmProvider};
use anyhow::Result;
use async_openai::{
    types::{ChatCompletionRequestMessage, ChatCompletionRequestMessageContentPart, CreateChatCompletionRequestArgs},
    Client,
};
use async_trait::async_trait;

/// OpenAI LLM provider for command resolution
pub struct OpenAiProvider {
    client: Client<async_openai::config::OpenAIConfig>,
    model: String,
}

impl OpenAiProvider {
    /// Create a new OpenAI provider
    ///
    /// # Arguments
    ///
    /// * `api_key` - OpenAI API key
    /// * `model` - Model name (e.g., "gpt-4", "gpt-3.5-turbo")
    pub fn new(api_key: String, model: String) -> Self {
        let client = Client::with_config(
            async_openai::config::OpenAIConfig::new()
                .with_api_key(api_key)
        );

        Self { client, model }
    }

    /// Generate the prompt for command resolution
    fn create_prompt(&self, query: &str, commands: &[CommandMetadata]) -> String {
        let mut prompt = format!(
            "You are a command-line interface assistant. The user has asked: \"{}\"\n\n",
            query
        );

        prompt.push_str("Available commands:\n");
        for cmd in commands {
            prompt.push_str(&format!("- {}: {}\n", cmd.id, cmd.summary));
            if let Some(syntax) = &cmd.syntax {
                prompt.push_str(&format!("  Syntax: {}\n", syntax));
            }
            if let Some(category) = &cmd.category {
                prompt.push_str(&format!("  Category: {}\n", category));
            }
            prompt.push('\n');
        }

        prompt.push_str(
            "Please respond with a JSON object containing:
- command_id: the exact ID of the command to execute
- args: object with positional (array) and named (object) arguments
- confidence: number between 0.0 and 1.0 indicating how confident you are
- reasoning: brief explanation of why you chose this command

Example response:
{
  \"command_id\": \"deploy\",
  \"args\": {
    \"positional\": [],
    \"named\": {\"env\": \"production\"}
  },
  \"confidence\": 0.95,
  \"reasoning\": \"The user wants to deploy to production environment\"
}

If no command matches the query, set confidence to 0.0 and command_id to \"\"."
        );

        prompt
    }

    /// Parse the LLM response into a CommandResolution
    fn parse_response(&self, response: &str) -> Result<CommandResolution> {
        // Try to extract JSON from the response
        let json_start = response.find('{').unwrap_or(0);
        let json_end = response.rfind('}').unwrap_or(response.len());
        let json_str = &response[json_start..=json_end];

        let parsed: serde_json::Value = serde_json::from_str(json_str)?;

        let command_id = parsed["command_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing or invalid command_id"))?
            .to_string();

        let confidence = parsed["confidence"]
            .as_f64()
            .ok_or_else(|| anyhow::anyhow!("Missing or invalid confidence"))?
            as f32;

        // If confidence is too low or command_id is empty, return an error
        if confidence < 0.3 || command_id.is_empty() {
            return Err(anyhow::anyhow!("Low confidence in command resolution"));
        }

        let args_obj = &parsed["args"];

        // Parse positional arguments
        let positional: Vec<String> = args_obj["positional"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        // Parse named arguments
        let named: std::collections::HashMap<String, String> = args_obj["named"]
            .as_object()
            .unwrap_or(&serde_json::Map::new())
            .iter()
            .filter_map(|(k, v)| {
                v.as_str().map(|s| (k.clone(), s.to_string()))
            })
            .collect();

        let args = CommandArgs { positional, named };

        let reasoning = parsed["reasoning"]
            .as_str()
            .map(|s| s.to_string());

        Ok(CommandResolution {
            command_id,
            args,
            confidence,
            reasoning,
        })
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn resolve_command(
        &self,
        query: &str,
        available_commands: &[CommandMetadata],
    ) -> Result<CommandResolution> {
        let prompt = self.create_prompt(query, available_commands);

        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.model)
            .messages(vec![
                ChatCompletionRequestMessage::User(
                    async_openai::types::ChatCompletionRequestUserMessage {
                        content: async_openai::types::ChatCompletionRequestUserMessageContent::Text(prompt),
                        ..Default::default()
                    }
                )
            ])
            .max_tokens(1000u16)
            .temperature(0.1) // Low temperature for consistent results
            .build()?;

        let response = self.client.chat().create(request).await?;
        let content = response.choices
            .first()
            .and_then(|choice| choice.message.content.as_ref())
            .ok_or_else(|| anyhow::anyhow!("No response from OpenAI"))?;

        self.parse_response(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prompt_creation() {
        let provider = OpenAiProvider::new("test-key".to_string(), "gpt-4".to_string());

        let commands = vec![
            CommandMetadata {
                id: "deploy".to_string(),
                summary: "Deploy application".to_string(),
                syntax: Some("deploy --env <env>".to_string()),
                category: Some("deployment".to_string()),
            }
        ];

        let prompt = provider.create_prompt("deploy to production", &commands);
        assert!(prompt.contains("deploy to production"));
        assert!(prompt.contains("deploy"));
        assert!(prompt.contains("Deploy application"));
    }

    #[test]
    fn test_response_parsing() {
        let provider = OpenAiProvider::new("test-key".to_string(), "gpt-4".to_string());

        let response = r#"{
            "command_id": "deploy",
            "args": {
                "positional": [],
                "named": {"env": "production"}
            },
            "confidence": 0.95,
            "reasoning": "User wants to deploy to production"
        }"#;

        let resolution = provider.parse_response(response).unwrap();
        assert_eq!(resolution.command_id, "deploy");
        assert_eq!(resolution.confidence, 0.95);
        assert_eq!(resolution.args.named.get("env"), Some(&"production".to_string()));
        assert_eq!(resolution.reasoning, Some("User wants to deploy to production".to_string()));
    }
}