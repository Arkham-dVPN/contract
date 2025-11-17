import * as bs58 from 'bs58';
import { writeFileSync } from 'fs';
import { resolve } from 'path';
import { Keypair } from '@solana/web3.js';

// The private key provided
const privateKeyBase58 = '48AmU3RybXWNsxkkWn8mZcANZsbxgTTPhDGC2bKJcgJg7wWZGAjjgQVEuEhwpta9vGZsfb9gTE2dwQDv7z1dgoYj';

// Decode the base58 private key
const privateKeyBytes = bs58.decode(privateKeyBase58);

// Convert to number array
const privateKeyArray = Array.from(privateKeyBytes);

console.log('Private key as byte array:');
console.log(JSON.stringify(privateKeyArray));

// Write to file in parent directory
const parentDir = resolve(__dirname, '..');
const walletPath = resolve(parentDir, '..', 'wallet.json');

writeFileSync(walletPath, JSON.stringify(privateKeyArray));
console.log(`\nWallet file created at: ${walletPath}`);

// Also verify the public key
const keypair = Keypair.fromSecretKey(Uint8Array.from(privateKeyArray));
console.log('Public key:', keypair.publicKey.toString());