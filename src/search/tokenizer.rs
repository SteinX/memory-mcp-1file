use bm_25::Tokenizer;
use std::collections::HashSet;

#[derive(Clone, Default)]
pub struct CodeTokenizer;

impl Tokenizer for CodeTokenizer {
    fn tokenize(&self, input_text: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut unique_tokens = HashSet::new(); // Deduplicate to avoid artificial TF inflation

        // Pass 1: split by non-alphanumeric
        for word in input_text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
        {
            // Keep the exact token (lowercase) - e.g. "OdooAuthService" -> "odooauthservice"
            let lower_word = word.to_lowercase();
            if unique_tokens.insert(lower_word.clone()) {
                tokens.push(lower_word);
            }

            let mut current_token = String::new();
            let mut prev_char_type = 0; // 1=lower, 2=upper, 3=digit
            let mut chars = word.chars().peekable();
            let mut has_split = false;

            while let Some(c) = chars.next() {
                let mut c_type = 0;
                if c.is_lowercase() {
                    c_type = 1;
                } else if c.is_uppercase() {
                    c_type = 2;
                } else if c.is_numeric() {
                    c_type = 3;
                }

                let mut split = false;
                if prev_char_type == 1 && c_type == 2 {
                    split = true; // aA -> a, A
                }
                if prev_char_type == 1 && c_type == 3 {
                    split = true; // a1 -> a, 1
                }
                if prev_char_type == 3 && c_type == 1 {
                    split = true; // 1a -> 1, a
                }

                // Handle Acronyms: HTTPRequest -> HTTP, Request
                if prev_char_type == 2 && c_type == 2 {
                    if let Some(&next_c) = chars.peek() {
                        if next_c.is_lowercase() {
                            split = true; // AAa -> A, Aa
                        }
                    }
                }

                if split && !current_token.is_empty() {
                    let sub_token = current_token.to_lowercase();
                    if unique_tokens.insert(sub_token.clone()) {
                        tokens.push(sub_token);
                    }
                    current_token.clear();
                    has_split = true;
                }
                current_token.push(c);
                if c_type != 0 {
                    prev_char_type = c_type;
                }
            }
            if has_split && !current_token.is_empty() {
                let sub_token = current_token.to_lowercase();
                if unique_tokens.insert(sub_token.clone()) {
                    tokens.push(sub_token);
                }
            }
        }
        tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_code_tokenizer() {
        let tokenizer = CodeTokenizer::default();

        // camelCase & PascalCase
        let tokens = tokenizer.tokenize("OdooAuthService");
        assert!(tokens.contains(&"odooauthservice".to_string()));
        assert!(tokens.contains(&"odoo".to_string()));
        assert!(tokens.contains(&"auth".to_string()));
        assert!(tokens.contains(&"service".to_string()));

        // snake_case
        let tokens = tokenizer.tokenize("my_variable_name");
        assert!(tokens.contains(&"my".to_string()));
        assert!(tokens.contains(&"variable".to_string()));
        assert!(tokens.contains(&"name".to_string()));

        // Numbers
        let tokens = tokenizer.tokenize("http2_server");
        assert!(tokens.contains(&"http".to_string()));
        assert!(tokens.contains(&"2".to_string()));
        assert!(tokens.contains(&"server".to_string()));

        // Acronyms
        let tokens = tokenizer.tokenize("XMLHttpRequest");
        assert!(tokens.contains(&"xml".to_string()));
        assert!(tokens.contains(&"http".to_string()));
        assert!(tokens.contains(&"request".to_string()));

        // Special characters
        let tokens = tokenizer.tokenize("fn main() {");
        assert!(tokens.contains(&"fn".to_string()));
        assert!(tokens.contains(&"main".to_string()));
    }
}
