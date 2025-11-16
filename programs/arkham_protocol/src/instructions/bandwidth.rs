use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    keccak,
    sysvar::instructions::{
        load_instruction_at_checked,
        ID as INSTRUCTIONS_SYSVAR_ID,
    },
    ed25519_program,
};

/// Verifies Ed25519 signatures by checking that an Ed25519Program instruction
/// was included in the same transaction.
/// 
/// This approach uses instruction introspection - the client MUST include
/// Ed25519Program verification instructions before calling this program.
/// 
/// # Security Model
/// - Client creates Ed25519Program.createInstructionWithPublicKey() for each signature
/// - These instructions are placed BEFORE the Arkham program instruction
/// - This function verifies those instructions exist and match our expected data
/// 
/// # Arguments
/// * `instructions_sysvar` - The Instructions sysvar account
/// * `message` - The message that was signed
/// * `signature` - The 64-byte Ed25519 signature
/// * `public_key` - The signer's public key
/// * `instruction_index` - Which instruction index to check (relative to current)
/// 
/// # Returns
/// * `Result<()>` - Ok if signature is valid via Ed25519Program, error otherwise
pub fn verify_ed25519_signature_via_sysvar(
    instructions_sysvar: &AccountInfo,
    message: &[u8],
    signature: &[u8; 64],
    public_key: &Pubkey,
    instruction_index: u16,
) -> Result<()> {
    // Verify we're actually looking at the Instructions sysvar
    require!(
        instructions_sysvar.key() == INSTRUCTIONS_SYSVAR_ID,
        BandwidthError::InvalidInstructionsSysvar
    );

    // Load the Ed25519Program instruction at the specified index
    let ed25519_ix = load_instruction_at_checked(
        instruction_index as usize,
        instructions_sysvar,
    ).map_err(|_| BandwidthError::Ed25519InstructionNotFound)?;

    // Verify it's actually an Ed25519Program instruction
    require!(
        ed25519_ix.program_id == ed25519_program::ID,
        BandwidthError::InvalidEd25519Instruction
    );

    // Parse the Ed25519Program instruction data
    // Format: [num_signatures: u8, padding: u8, signature_offset: u16, 
    //          signature_instruction_index: u16, public_key_offset: u16,
    //          public_key_instruction_index: u16, message_data_offset: u16,
    //          message_data_size: u16, message_instruction_index: u16,
    //          ...signature(64), ...pubkey(32), ...message]
    
    let data = &ed25519_ix.data;
    require!(
        data.len() >= 2 + 5*2 + 64 + 32 + message.len(),
        BandwidthError::InvalidEd25519Data
    );

    // Extract signature from instruction data (starts at byte 14)
    let sig_start = 14;
    let sig_end = sig_start + 64;
    let ix_signature = &data[sig_start..sig_end];
    
    // Extract public key (starts after signature)
    let pk_start = sig_end;
    let pk_end = pk_start + 32;
    let ix_pubkey = &data[pk_start..pk_end];
    
    // Extract message (starts after public key)
    let msg_start = pk_end;
    let msg_end = msg_start + message.len();
    require!(
        data.len() >= msg_end,
        BandwidthError::InvalidEd25519Data
    );
    let ix_message = &data[msg_start..msg_end];

    // Verify the signature matches what we expect
    require!(
        ix_signature == signature,
        BandwidthError::SignatureMismatch
    );

    // Verify the public key matches
    require!(
        ix_pubkey == public_key.to_bytes().as_ref(),
        BandwidthError::PublicKeyMismatch
    );

    // Verify the message matches
    require!(
        ix_message == message,
        BandwidthError::MessageMismatch
    );

    // If we get here, the Ed25519Program instruction exists and matches our data
    // The Ed25519Program already verified the signature cryptographically
    Ok(())
}

/// Simplified wrapper that verifies both Seeker and Warden signatures
/// 
/// # Arguments
/// * `instructions_sysvar` - The Instructions sysvar account
/// * `message` - The bandwidth proof message
/// * `seeker_signature` - Seeker's signature
/// * `seeker_pubkey` - Seeker's public key
/// * `warden_signature` - Warden's signature
/// * `warden_pubkey` - Warden's public key
/// * `current_instruction_index` - The current instruction's index in the transaction
/// 
/// # Expected Transaction Layout
/// ```
/// Instruction 0: Ed25519Program (verify Seeker signature)
/// Instruction 1: Ed25519Program (verify Warden signature)
/// Instruction 2: ArkhamProtocol::submit_bandwidth_proof (this instruction)
/// ```
pub fn verify_dual_signatures(
    instructions_sysvar: &AccountInfo,
    message: &[u8],
    seeker_signature: &[u8; 64],
    seeker_pubkey: &Pubkey,
    warden_signature: &[u8; 64],
    warden_pubkey: &Pubkey,
) -> Result<()> {
    // Seeker's Ed25519 instruction should be 2 instructions before current (index -2)
    // Warden's Ed25519 instruction should be 1 instruction before current (index -1)
    
    // Note: We can't use negative indices, so we need to know the current instruction index
    // For now, we'll assume instructions 0 and 1 are the Ed25519 verifications
    
    verify_ed25519_signature_via_sysvar(
        instructions_sysvar,
        message,
        seeker_signature,
        seeker_pubkey,
        0, // First Ed25519 instruction
    )?;

    verify_ed25519_signature_via_sysvar(
        instructions_sysvar,
        message,
        warden_signature,
        warden_pubkey,
        1, // Second Ed25519 instruction
    )?;

    Ok(())
}

/// Creates a deterministic message for bandwidth proof signing
/// 
/// Both Seeker and Warden must sign this exact message to create a valid proof.
/// The message includes:
/// - Connection PDA (ensures proof is for specific connection)
/// - Megabytes consumed (the bandwidth amount being claimed)
/// - Timestamp (prevents replay attacks)
/// 
/// # Arguments
/// * `connection_pubkey` - The Connection account's public key
/// * `mb_consumed` - Amount of bandwidth in megabytes
/// * `timestamp` - Unix timestamp of the proof
/// 
/// # Returns
/// * `Vec<u8>` - The deterministic message bytes to be signed
pub fn create_proof_message(
    connection_pubkey: &Pubkey,
    mb_consumed: u64,
    timestamp: i64,
) -> Vec<u8> {
    let mut message = Vec::new();
    
    // Add connection pubkey (32 bytes)
    message.extend_from_slice(&connection_pubkey.to_bytes());
    
    // Add mb_consumed (8 bytes, little-endian)
    message.extend_from_slice(&mb_consumed.to_le_bytes());
    
    // Add timestamp (8 bytes, little-endian)
    message.extend_from_slice(&timestamp.to_le_bytes());
    
    // Hash the combined data for a fixed-size message
    // This also provides additional security against length extension attacks
    let hash = keccak::hash(&message);
    
    hash.to_bytes().to_vec()
}

/// Validates a bandwidth proof against expected constraints
/// 
/// Checks that:
/// - Bandwidth amount is reasonable (not zero, not impossibly large)
/// - Timestamp is recent (within last hour)
/// - Signatures are present and correct length
/// 
/// # Arguments
/// * `mb_consumed` - Amount of bandwidth claimed
/// * `timestamp` - When the bandwidth was measured
/// * `current_timestamp` - Current blockchain time
/// * `seeker_signature` - Seeker's signature bytes
/// * `warden_signature` - Warden's signature bytes
/// 
/// # Returns
/// * `Result<()>` - Ok if proof is valid, error with reason otherwise
pub fn validate_bandwidth_proof(
    mb_consumed: u64,
    timestamp: i64,
    current_timestamp: i64,
    seeker_signature: &[u8; 64],
    warden_signature: &[u8; 64],
) -> Result<()> {
    // 1. Validate bandwidth amount is reasonable
    require!(
        mb_consumed > 0,
        BandwidthError::ZeroBandwidth
    );
    
    require!(
        mb_consumed <= 10_000, // Max 10 GB per proof (prevents gaming)
        BandwidthError::ExcessiveBandwidth
    );
    
    // 2. Validate timestamp is recent (within last hour)
    const MAX_PROOF_AGE: i64 = 3600; // 1 hour in seconds
    let age = current_timestamp
        .checked_sub(timestamp)
        .ok_or(BandwidthError::InvalidTimestamp)?;
    
    require!(
        age >= 0 && age <= MAX_PROOF_AGE,
        BandwidthError::ProofTooOld
    );
    
    // 3. Validate signatures are not empty (basic sanity check)
    require!(
        seeker_signature != &[0u8; 64],
        BandwidthError::InvalidSignature
    );
    
    require!(
        warden_signature != &[0u8; 64],
        BandwidthError::InvalidSignature
    );
    
    Ok(())
}

/// Calculates expected bandwidth based on historical average
pub fn calculate_expected_bandwidth(
    historical_proofs: &[u64],
    window_size: usize,
) -> u64 {
    if historical_proofs.is_empty() {
        return 0;
    }
    
    let window_size = window_size.min(historical_proofs.len());
    let recent_proofs = &historical_proofs[historical_proofs.len() - window_size..];
    
    let sum: u64 = recent_proofs.iter().sum();
    sum / (window_size as u64)
}

/// Detects anomalous bandwidth claims
pub fn detect_bandwidth_anomaly(
    claimed_mb: u64,
    expected_mb: u64,
    threshold_multiplier: f64,
) -> bool {
    if expected_mb == 0 {
        return false;
    }
    
    let threshold = (expected_mb as f64 * threshold_multiplier) as u64;
    claimed_mb > threshold
}

/// Hashes a complete bandwidth proof for duplicate detection
pub fn hash_bandwidth_proof(
    connection: &Pubkey,
    mb_consumed: u64,
    timestamp: i64,
    seeker_sig: &[u8; 64],
    warden_sig: &[u8; 64],
) -> [u8; 32] {
    let mut data = Vec::new();
    
    data.extend_from_slice(&connection.to_bytes());
    data.extend_from_slice(&mb_consumed.to_le_bytes());
    data.extend_from_slice(&timestamp.to_le_bytes());
    data.extend_from_slice(seeker_sig);
    data.extend_from_slice(warden_sig);
    
    let hash = keccak::hash(&data);
    hash.to_bytes()
}

// Custom error codes specific to bandwidth validation
#[error_code]
pub enum BandwidthError {
    #[msg("Bandwidth amount cannot be zero")]
    ZeroBandwidth,
    
    #[msg("Bandwidth amount exceeds maximum allowed per proof")]
    ExcessiveBandwidth,
    
    #[msg("Proof timestamp is invalid")]
    InvalidTimestamp,
    
    #[msg("Proof is too old and cannot be accepted")]
    ProofTooOld,
    
    #[msg("Invalid or empty signature provided")]
    InvalidSignature,
    
    #[msg("Bandwidth claim appears anomalous and may be fraudulent")]
    AnomalousBandwidth,

    // Ed25519 verification errors
    #[msg("Invalid Instructions sysvar account")]
    InvalidInstructionsSysvar,
    
    #[msg("Ed25519Program instruction not found at expected index")]
    Ed25519InstructionNotFound,
    
    #[msg("Instruction is not an Ed25519Program instruction")]
    InvalidEd25519Instruction,
    
    #[msg("Ed25519Program instruction data is invalid or too short")]
    InvalidEd25519Data,
    
    #[msg("Signature in Ed25519 instruction doesn't match expected signature")]
    SignatureMismatch,
    
    #[msg("Public key in Ed25519 instruction doesn't match expected public key")]
    PublicKeyMismatch,
    
    #[msg("Message in Ed25519 instruction doesn't match expected message")]
    MessageMismatch,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_create_proof_message() {
        let connection = Pubkey::new_unique();
        let mb_consumed = 100u64;
        let timestamp = 1234567890i64;
        
        let message = create_proof_message(&connection, mb_consumed, timestamp);
        let message2 = create_proof_message(&connection, mb_consumed, timestamp);
        assert_eq!(message, message2);
        
        let message3 = create_proof_message(&connection, mb_consumed + 1, timestamp);
        assert_ne!(message, message3);
    }
    
    #[test]
    fn test_calculate_expected_bandwidth() {
        let proofs = vec![100, 110, 105, 95, 100];
        let expected = calculate_expected_bandwidth(&proofs, 5);
        assert_eq!(expected, 102);
        
        let expected = calculate_expected_bandwidth(&proofs, 3);
        assert_eq!(expected, 100);
    }
    
    #[test]
    fn test_detect_bandwidth_anomaly() {
        assert!(!detect_bandwidth_anomaly(105, 100, 2.0));
        assert!(detect_bandwidth_anomaly(300, 100, 2.0));
        assert!(!detect_bandwidth_anomaly(200, 100, 2.0));
    }
    
    #[test]
    fn test_hash_bandwidth_proof() {
        let connection = Pubkey::new_unique();
        let mb = 100u64;
        let ts = 1234567890i64;
        let sig1 = [1u8; 64];
        let sig2 = [2u8; 64];
        
        let hash1 = hash_bandwidth_proof(&connection, mb, ts, &sig1, &sig2);
        let hash2 = hash_bandwidth_proof(&connection, mb, ts, &sig1, &sig2);
        assert_eq!(hash1, hash2);
        
        let hash3 = hash_bandwidth_proof(&connection, mb, ts, &sig2, &sig1);
        assert_ne!(hash1, hash3);
    }
}