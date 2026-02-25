fn tokenize_polyglot(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    
    // Pass 1: split by any non-alphanumeric character
    for word in text.split(|c: char| !c.is_alphanumeric()).filter(|s| !s.is_empty()) {
        let mut current_token = String::new();
        let mut prev_char_type = 0; // 1=lower, 2=upper, 3=digit
        let mut chars = word.chars().peekable();
        
        while let Some(c) = chars.next() {
            let mut c_type = 0;
            if c.is_lowercase() { c_type = 1; }
            else if c.is_uppercase() { c_type = 2; }
            else if c.is_numeric() { c_type = 3; }
            
            let mut split = false;
            if prev_char_type == 1 && c_type == 2 { split = true; } // aA
            if prev_char_type == 1 && c_type == 3 { split = true; } // a1
            if prev_char_type == 3 && c_type == 1 { split = true; } // 1a
            if prev_char_type == 3 && c_type == 2 { split = true; } // 1A
            
            if prev_char_type == 2 && c_type == 2 {
                if let Some(&next_c) = chars.peek() {
                    if next_c.is_lowercase() { split = true; } // AAa
                }
            }
            
            if split && !current_token.is_empty() {
                tokens.push(current_token.to_lowercase());
                current_token.clear();
            }
            
            current_token.push(c);
            if c_type != 0 { prev_char_type = c_type; }
        }
        
        if !current_token.is_empty() {
            tokens.push(current_token.to_lowercase());
        }
    }
    
    tokens
}

fn main() {
    let cases = vec![
        "camelCaseFunction",
        "snake_case_variable",
        "PascalCaseClass",
        "kebab-case-id",
        "special_chars123$",
        "HTTPRequestType",
        "HTML5Parser",
        "parseXMLContent",
        "function(myArg) { return my_arg_2 + 1; }"
    ];
    for case in cases {
        println!("Input: {}", case);
        println!("  Tokens: {:?}", tokenize_polyglot(case));
    }
}
