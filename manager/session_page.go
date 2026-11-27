package main

import (
	"html"
	"io"
	"net/http"
	"regexp"
	"strconv"
	"strings"
)

var desktopHeadEnd = regexp.MustCompile(`(?i)</head\s*>`)
var desktopTitle = regexp.MustCompile(`(?is)<title(?:\s[^>]*)?>(.*?)</title\s*>`)

func decorateDesktopPage(response *http.Response, name string, preview, managed bool) error {
	if response.StatusCode != http.StatusOK || !strings.Contains(response.Header.Get("Content-Type"), "text/html") || response.Header.Get("Content-Encoding") != "" {
		return nil
	}
	const limit = 2 << 20
	body, err := io.ReadAll(io.LimitReader(response.Body, limit+1))
	response.Body.Close()
	if err != nil {
		return err
	}
	if len(body) > limit {
		return http.ErrContentLength
	}
	text := string(body)
	if head := desktopHeadEnd.FindStringIndex(text); head != nil {
		index := head[0]
		var extra string
		if name = strings.TrimSpace(name); name != "" && !strings.Contains(text, "data-lightos-remote-desktop-title") {
			title := html.EscapeString(name)
			if current := desktopTitle.FindStringSubmatchIndex(text[:index]); current != nil {
				start, end := current[2], current[3]
				text = text[:start] + title + text[end:]
				index += len(title) - (end - start)
			} else {
				extra += "<title>" + title + "</title>"
			}
			extra += `<meta name="lightos-desktop-name" content="` + title + `"><script src="/session-title.js"></script>`
		}
		if managed {
			extra += `<meta name="lazycat-managed-desktop" content="true">`
		}
		if preview {
			extra += `<script src="/session-preview.js"></script>`
		}
		text = text[:index] + extra + text[index:]
	}
	response.Body = io.NopCloser(strings.NewReader(text))
	response.ContentLength = int64(len(text))
	response.Header.Set("Content-Length", strconv.Itoa(len(text)))
	response.Header.Del("ETag")
	return nil
}
