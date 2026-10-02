package protocol

import (
	"bytes"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"unicode/utf8"
)

const maxJSONDepth = 64
const maxJSONTokens = MaxFrameBytes

func DecodeStrict(data []byte, target any) error {
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(target); err != nil {
		return err
	}
	var extra any
	if err := decoder.Decode(&extra); !errors.Is(err, io.EOF) {
		return fmt.Errorf("trailing JSON value")
	}
	return nil
}

// ReadFrame admits the declared size before allocation, rejects duplicate keys
// recursively, and requires EOF: one child process handles exactly one request.
func ReadFrame[T any](reader io.Reader) (T, error) {
	var result T
	var header [8]byte
	if _, err := io.ReadFull(reader, header[:]); err != nil {
		return result, err
	}
	size := binary.LittleEndian.Uint64(header[:])
	if size > MaxFrameBytes {
		return result, fmt.Errorf("frame exceeds %d byte cap", MaxFrameBytes)
	}
	body := make([]byte, int(size))
	if _, err := io.ReadFull(reader, body); err != nil {
		return result, err
	}
	if !utf8.Valid(body) {
		return result, fmt.Errorf("protocol JSON must be UTF-8")
	}
	var trailing [1]byte
	if n, err := reader.Read(trailing[:]); n != 0 || !errors.Is(err, io.EOF) {
		return result, fmt.Errorf("one request and EOF required")
	}
	decoder := json.NewDecoder(bytes.NewReader(body))
	decoder.UseNumber()
	tokens := 0
	if err := walkJSON(decoder, 0, &tokens); err != nil {
		return result, err
	}
	err := DecodeStrict(body, &result)
	return result, err
}

func walkJSON(decoder *json.Decoder, depth int, count *int) error {
	if depth > maxJSONDepth || *count >= maxJSONTokens {
		return fmt.Errorf("JSON structure exceeds protocol bound")
	}
	token, err := decoder.Token()
	if err != nil {
		return err
	}
	*count++
	delimiter, nested := token.(json.Delim)
	if !nested {
		return nil
	}
	switch delimiter {
	case '{':
		keys := map[string]struct{}{}
		for decoder.More() {
			token, err := decoder.Token()
			if err != nil {
				return err
			}
			key, ok := token.(string)
			if !ok || !wireField(key, depth) {
				return fmt.Errorf("object member must use its canonical protocol spelling")
			}
			if _, duplicate := keys[key]; duplicate {
				return fmt.Errorf("duplicate JSON field %q", key)
			}
			keys[key] = struct{}{}
			*count++
			if err := walkJSON(decoder, depth+1, count); err != nil {
				return err
			}
		}
		closing, err := decoder.Token()
		if err != nil || closing != json.Delim('}') {
			return fmt.Errorf("unterminated JSON object")
		}
	case '[':
		for decoder.More() {
			if err := walkJSON(decoder, depth+1, count); err != nil {
				return err
			}
		}
		closing, err := decoder.Token()
		if err != nil || closing != json.Delim(']') {
			return fmt.Errorf("unterminated JSON array")
		}
	default:
		return fmt.Errorf("unexpected JSON delimiter")
	}
	return nil
}

// Responses are constructed from bounded definitions/chunks/diagnostics only.
// The encoder still checks the resulting complete frame before writing any part.
func WriteFrame(writer io.Writer, value any) error {
	body, err := json.Marshal(value)
	if err != nil {
		return err
	}
	if len(body) > MaxFrameBytes {
		return fmt.Errorf("encoded response exceeds frame cap")
	}
	var header [8]byte
	binary.LittleEndian.PutUint64(header[:], uint64(len(body)))
	if err := writeAll(writer, header[:]); err != nil {
		return err
	}
	return writeAll(writer, body)
}
func writeAll(writer io.Writer, bytes []byte) error {
	for len(bytes) != 0 {
		count, err := writer.Write(bytes)
		if err != nil {
			return err
		}
		if count == 0 {
			return io.ErrShortWrite
		}
		bytes = bytes[count:]
	}
	return nil
}

// Request members use snake_case. Go's JSON decoder also accepts case-folded
// aliases; reject those before decoding so a second spelling cannot overwrite
// a Rust-recognized authority field. Only response variant tags use capitals.
func wireField(key string, depth int) bool {
	if depth == 0 {
		switch key {
		case "Prepared", "Accepted", "Event", "Exported", "Done", "Failed", "Cancelled":
			return true
		}
	}
	if key == "" {
		return false
	}
	for i := range len(key) {
		c := key[i]
		if !(c >= 'a' && c <= 'z' || c >= '0' && c <= '9' || c == '_') {
			return false
		}
	}
	return true
}
