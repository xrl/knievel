{{/*
Expand the name of the chart.
*/}}
{{- define "knievel.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/*
Create a default fully qualified app name.
We truncate at 63 chars because some Kubernetes name fields are
limited to that (DNS labels). Mirrors the helm `create` template.
*/}}
{{- define "knievel.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{/*
Chart label.
*/}}
{{- define "knievel.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/*
Common labels — every resource carries the standard
`app.kubernetes.io/*` set so kubectl selectors and Prometheus
relabel rules work out of the box.
*/}}
{{- define "knievel.labels" -}}
helm.sh/chart: {{ include "knievel.chart" . }}
{{ include "knievel.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{/*
Selector labels — the strict subset used to match Pods. These
must NOT include version-y labels (immutable on Deployment
selectors).
*/}}
{{- define "knievel.selectorLabels" -}}
app.kubernetes.io/name: {{ include "knievel.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{/*
Service-account name. Returns the override if set, else the
fullname-derived default.
*/}}
{{- define "knievel.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "knievel.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{/*
Image reference. Honors a digest in `image.tag` (`sha256:...`) by
using `@` instead of `:`. Falls back to `.Chart.AppVersion` when
`image.tag` is empty.
*/}}
{{- define "knievel.image" -}}
{{- $tag := default .Chart.AppVersion .Values.image.tag -}}
{{- if hasPrefix "sha256:" $tag -}}
{{- printf "%s@%s" .Values.image.repository $tag -}}
{{- else -}}
{{- printf "%s:%s" .Values.image.repository $tag -}}
{{- end -}}
{{- end -}}

{{/*
Database URL builder. Composes the postgres URL using the
existingSecret-projected env vars; the rendered config.yaml
references it via `${KNIEVEL_DATABASE_URL}`.
*/}}
{{- define "knievel.databaseUrl" -}}
postgres://${KNIEVEL_DB_USER}:${KNIEVEL_DB_PASSWORD}@{{ .Values.database.host }}:{{ .Values.database.port }}/{{ .Values.database.name }}?sslmode={{ .Values.database.sslMode }}
{{- end -}}
