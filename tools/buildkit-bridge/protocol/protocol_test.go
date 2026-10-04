package protocol

import (
	"bytes"
	"encoding/binary"
	"strings"
	"testing"
)

func framed(body string) []byte {
	result := make([]byte, 8+len(body))
	binary.LittleEndian.PutUint64(result, uint64(len(body)))
	copy(result[8:], body)
	return result
}

const fileRequest = `{"protocol_version":3,"session":"fixture","attempt":2,"epoch":3,"plan":{"platform":{"os":"linux","architecture":"amd64"},"nodes":[{"kind":"file","input":null,"path":"/hello","data":"cGF5bG9hZA==","mode":292}],"root":0,"exporter":{"kind":"local"}}}`

func TestReadFrameRetainsTheAdmittedIdentityAndContent(t *testing.T) {
	request, err := ReadFrame[LowerRequest](bytes.NewReader(framed(fileRequest)))
	if err != nil {
		t.Fatal(err)
	}
	if request.Session != "fixture" || request.Attempt != 2 || request.Epoch != 3 || len(request.Plan.Nodes) != 1 {
		t.Fatalf("decoded request lost its operation identity: %+v", request)
	}
	file := request.Plan.Nodes[0].File
	if file == nil || string(file.Data) != "payload" || file.Path != "/hello" || file.Mode != 0444 {
		t.Fatalf("decoded file semantics differ: %+v", file)
	}
}
func TestDecoderRejectsAmbiguousOrTruncatedRequests(t *testing.T) {
	cases := map[string][]byte{
		"duplicate identity":     framed(strings.Replace(fileRequest, `"epoch":3`, `"epoch":3,"epoch":4`, 1)),
		"duplicate nested mode":  framed(strings.Replace(fileRequest, `"mode":292`, `"mode":292,"mode":493`, 1)),
		"unknown path authority": framed(strings.Replace(fileRequest, `"root":0`, `"root":0,"host_root":"/"`, 1)),
		"missing root":           framed(strings.Replace(fileRequest, `,"root":0`, "", 1)),
		"null root":              framed(strings.Replace(fileRequest, `"root":0`, `"root":null`, 1)),
		"integer overflow":       framed(strings.Replace(fileRequest, `"attempt":2`, `"attempt":18446744073709551616`, 1)),
		"negative index":         framed(strings.Replace(fileRequest, `"root":0`, `"root":-1`, 1)),
		"truncated header":       {1, 2, 3},
		"truncated body":         framed(fileRequest)[:len(fileRequest)],
		"trailing frame":         append(framed(fileRequest), framed(fileRequest)...),
	}
	oversized := make([]byte, 8)
	binary.LittleEndian.PutUint64(oversized, MaxFrameBytes+1)
	cases["oversized before body"] = oversized
	for name, input := range cases {
		t.Run(name, func(t *testing.T) {
			if _, err := ReadFrame[LowerRequest](bytes.NewReader(input)); err == nil {
				t.Fatal("invalid request was admitted")
			}
		})
	}
}
func TestPlanRejectsCyclesAndUnreachableProduction(t *testing.T) {
	for _, body := range []string{
		strings.Replace(fileRequest, `"input":null`, `"input":0`, 1),
		strings.Replace(fileRequest, `}],"root":0`, `},{"kind":"file","input":null,"path":"/unused","data":"","mode":292}],"root":0`, 1),
		strings.Replace(fileRequest, `"session":"fixture"`, `"session":"../escape"`, 1),
		strings.Replace(fileRequest, `"epoch":3`, `"epoch":0`, 1),
	} {
		request, err := ReadFrame[LowerRequest](bytes.NewReader(framed(body)))
		if err != nil {
			t.Fatalf("fixture did not reach semantic admission: %v", err)
		}
		if err := request.Validate(); err == nil {
			t.Fatal("invalid graph/identity reached lowering")
		}
	}
}
