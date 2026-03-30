const OTPAuth = require('otpauth');
const secretStr = "14N2srgWDM72pZFiwi8D3RxVR59XIw+rClILk43968F93l3yz4ejI808oXhYlWPQ";

// Method 1: remove invalid base32 characters
const stripped = secretStr.toUpperCase().replace(/[^A-Z2-7]/g, '');
const totp = new OTPAuth.TOTP({ secret: OTPAuth.Secret.fromBase32(stripped) });
console.log("Stripped secret:", stripped);
console.log("Token:", totp.generate());

