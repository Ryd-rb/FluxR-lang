package main

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"strconv"
	"time"
)

var jwtHeader = base64.RawURLEncoding.EncodeToString([]byte(`{"alg":"HS256","typ":"JWT"}`))

// mintToken returns an HS256 JWT with claims {"sub": "<id>", "iat": now, "exp": now + 30 days} (SPEC.md),
// i.e. the token a returning user's browser would have saved.
func mintToken(secret []byte, userID int64, now time.Time) string {
	iat := now.Unix()
	claims := `{"sub":"` + strconv.FormatInt(userID, 10) + `","iat":` + strconv.FormatInt(iat, 10) +
		`,"exp":` + strconv.FormatInt(iat+30*24*3600, 10) + `}`
	msg := jwtHeader + "." + base64.RawURLEncoding.EncodeToString([]byte(claims))
	mac := hmac.New(sha256.New, secret)
	mac.Write([]byte(msg))
	return msg + "." + base64.RawURLEncoding.EncodeToString(mac.Sum(nil))
}
