import { StrictMode, useState, useEffect, useMemo, useRef } from 'react';
import { createRoot } from 'react-dom/client';
import hljs from 'highlight.js/lib/core';
import 'highlight.js/styles/hybrid.css';
import styles from './api.module.css';

hljs.registerLanguage('json', require('highlight.js/lib/languages/json'));
hljs.registerLanguage(
  'plaintext',
  require('highlight.js/lib/languages/plaintext'),
);

const CodeBlock = ({
  value,
  language,
  originalValue,
}: { value: string; language?: string | null; originalValue?: string }) => {
  const codeRef = useRef<HTMLPreElement>(null);
  const langClass = language ? `language-${language}` : undefined;

  // biome-ignore lint/correctness/useExhaustiveDependencies:
  useEffect(() => {
    if (codeRef.current && !codeRef.current.dataset.highlighted) {
      hljs.highlightElement(codeRef.current);
    }
  }, [value]);

  const copyToClipboard = () => {
    navigator.clipboard.writeText(originalValue || value).catch((e) => {
      console.error('Failed to copy text', e);
    });
  };

  return (
    <pre className={styles.codeBlock}>
      <button
        className={`secondary ${styles.copyButton} icon-copy`}
        onClick={copyToClipboard}
      />
      <code ref={codeRef} className={langClass}>
        {value}
      </code>
    </pre>
  );
};

type ProjectsResponse = {
  projects: ProjectResponse[];
};

type ProjectResponse = {
  id: string;
  owner: string;
  repo: string;
  name: string | null;
  short_name: string | null;
  platform: string | null;
  default_version: string;
  report_versions: string[];
  report_categories: CategoryResponse[];
};

type CategoryResponse = {
  id: string;
  name: string;
};

const fetchProjects = async () => {
  const url = new URL('/projects', window.location.origin);
  url.searchParams.append('sort', 'name');
  const response = await fetch(url, {
    headers: { Accept: 'application/json' },
  });
  if (!response.ok) {
    throw new Error('Failed to fetch projects');
  }
  return (await response.json()) as ProjectsResponse;
};

const fetchProjectById = async (id: string, version: string | null) => {
  const url = new URL(`/projects/${id}`, window.location.origin);
  if (version) {
    url.searchParams.append('version', version);
  }
  const response = await fetch(url, {
    headers: { Accept: 'application/json' },
  });
  if (!response.ok) {
    throw new Error('Failed to fetch projects');
  }
  return (await response.json()) as ProjectResponse;
};

const fetchApiUrl = async (url: string) => {
  const response = await fetch(url, {
    headers: { Accept: 'application/json' },
  });
  if (!response.ok) {
    throw new Error('Failed to fetch projects');
  }
  return await response.text();
};

const ProjectForm = () => {
  const [projects, setProjects] = useState<ProjectResponse[]>([]);
  const [loadingProjects, setLoadingProjects] = useState(true);
  const [loadingProject, setLoadingProject] = useState(true);

  type Selected = {
    project: string | null;
    version: string | null;
    category: string | null;
  };
  const [selected, setSelected] = useState<Selected>({
    project: null,
    version: null,
    category: null,
  });
  const [mode, setMode] = useState('overview');
  const [format, setFormat] = useState('json');
  const [formatOptions, setFormatOptions] = useState<any>({});
  const [useProjectId, setUseProjectId] = useState(false);

  const [currentProject, setCurrentProject] = useState<ProjectResponse | null>(
    null,
  );

  useEffect(() => {
    setLoadingProjects(true);
    fetchProjects()
      .then((response) => {
        setProjects(response.projects);
        const url = new URL(document.location.href);
        const deferredProject = url.searchParams.get('project');
        if (deferredProject) {
          const project = response.projects.find(
            (p) => p.id.toString() === deferredProject,
          );
          if (project) {
            setCurrentProject(project);
            setSelected({
              project: project.id,
              version: null,
              category: null,
            });
          }
        }
        setLoadingProjects(false);
      })
      .catch((error) => {
        console.error('Error fetching projects:', error);
        setProjects([]);
        setLoadingProjects(false);
      });
  }, []);

  useEffect(() => {
    if (selected.project === null) {
      return;
    }
    setLoadingProject(true);
    fetchProjectById(selected.project, selected.version)
      .then((response) => {
        setCurrentProject(response);
        setLoadingProject(false);
      })
      .catch((error) => {
        console.error('Error fetching project by ID:', error);
        setCurrentProject(null);
        setLoadingProject(false);
      });
  }, [selected.project, selected.version]);

  const url = new URL('/', window.location.origin);
  url.searchParams.append('mode', mode);
  if (currentProject) {
    if (useProjectId) {
      url.pathname = `/projects/${currentProject.id}`;
      if (selected.version) {
        url.searchParams.append('version', selected.version);
      }
    } else {
      url.pathname = `/${currentProject.owner}/${currentProject.repo}`;
      if (selected.version) {
        url.pathname += `/${selected.version || currentProject.default_version}`;
      }
    }
    if (selected.category) {
      url.searchParams.append('category', selected.category);
    }
  }
  url.pathname += `.${format}`;
  if (mode === 'shield') {
    if (formatOptions.label) {
      url.searchParams.append('label', formatOptions.label);
    }
    if (formatOptions.labelColor) {
      url.searchParams.append('labelColor', formatOptions.labelColor);
    }
    if (formatOptions.color) {
      url.searchParams.append('color', formatOptions.color);
    }
    if (formatOptions.style) {
      url.searchParams.append('style', formatOptions.style);
    }
    if (formatOptions.measure) {
      url.searchParams.append('measure', formatOptions.measure);
    }
  }

  const updateMode = (newMode: string) => {
    setMode(newMode);
    setFormat(formatsByMode[newMode][0]);
    setFormatOptions({});
  };

  const updateFormat = (newFormat: string) => {
    setFormat(newFormat);
  };

  let modeOptions = null;
  if (mode === 'shield') {
    modeOptions = (
      <>
        <div className="grid">
          <label>
            Measure
            <select
              name="measure"
              value={formatOptions.measure || ''}
              onChange={(e) =>
                setFormatOptions((existing: any) => ({
                  ...existing,
                  measure: e.target.value,
                }))
              }
            >
              <option value="">Default</option>
              <option value="fuzzy_match_percent">Fuzzy Match (Percent)</option>
              <option value="matched_code_percent">
                Matched Code (Percent)
              </option>
              <option value="matched_code_bytes">Matched Code (Bytes)</option>
              <option value="matched_code_size">Matched Code (Size)</option>
              <option value="matched_data_percent">
                Matched Data (Percent)
              </option>
              <option value="matched_data_bytes">Matched Data (Bytes)</option>
              <option value="matched_data_size">Matched Data (Size)</option>
              <option value="matched_functions">
                Matched Functions (Count)
              </option>
              <option value="matched_functions_percent">
                Matched Functions (Percent)
              </option>
              <option value="complete_code_percent">
                Linked Code (Percent)
              </option>
              <option value="complete_code_bytes">Linked Code (Bytes)</option>
              <option value="complete_code_size">Linked Code (Size)</option>
              <option value="complete_data_percent">
                Linked Data (Percent)
              </option>
              <option value="complete_data_bytes">Linked Data (Bytes)</option>
              <option value="complete_data_size">Linked Data (Size)</option>
              <option value="complete_units">Linked Units (Count)</option>
              <option value="complete_units_percent">
                Linked Units (Percent)
              </option>
            </select>
          </label>
        </div>
        <div className="grid">
          <label>
            Style
            <select
              name="style"
              value={formatOptions.style || ''}
              onChange={(e) =>
                setFormatOptions((existing: any) => ({
                  ...existing,
                  style: e.target.value,
                }))
              }
            >
              <option value="">Default</option>
              <option value="flat">Flat</option>
              <option value="plastic">Plastic</option>
              <option value="flatsquare">Flat Square</option>
            </select>
          </label>
          <label>
            Color
            <div className={styles.clearContainer}>
              <input
                name="color"
                type="color"
                value={formatOptions.color || ''}
                onChange={(e) =>
                  setFormatOptions((existing: any) => ({
                    ...existing,
                    color: e.target.value,
                  }))
                }
              />
              <button
                className="secondary icon-cancel"
                onClick={() =>
                  setFormatOptions((existing: any) => ({
                    ...existing,
                    color: null,
                  }))
                }
              />
            </div>
          </label>
        </div>
        <div className="grid">
          <label>
            Label
            <input
              name="label"
              type="text"
              value={formatOptions.label || ''}
              onChange={(e) =>
                setFormatOptions((existing: any) => ({
                  ...existing,
                  label: e.target.value,
                }))
              }
            />
          </label>
          <label>
            Label Color
            <div className={styles.clearContainer}>
              <input
                name="labelColor"
                type="color"
                value={formatOptions.labelColor || ''}
                onChange={(e) =>
                  setFormatOptions((existing: any) => ({
                    ...existing,
                    labelColor: e.target.value,
                  }))
                }
              />
              <button
                className="secondary icon-cancel"
                onClick={() =>
                  setFormatOptions((existing: any) => ({
                    ...existing,
                    labelColor: null,
                  }))
                }
              />
            </div>
          </label>
        </div>
      </>
    );
  }

  return (
    <div>
      <div className="grid">
        <label>
          Project
          <select
            name="project"
            value={selected.project || ''}
            disabled={loadingProjects}
            onChange={(e) =>
              setSelected({
                project: e.target.value,
                version: null,
                category: null,
              })
            }
          >
            <option value="">Select a project</option>
            {projects.map((project) => (
              <option key={project.id} value={project.id}>
                {project.name}
              </option>
            ))}
          </select>
        </label>
        <label>
          Version
          <select
            name="version"
            value={selected.version || ''}
            disabled={loadingProject || !currentProject}
            onChange={(e) =>
              setSelected((existing) => ({
                project: existing.project,
                version: e.target.value,
                category: null,
              }))
            }
          >
            <option value="">Default</option>
            {currentProject?.report_versions.map((version) => (
              <option key={version} value={version}>
                {version}
              </option>
            ))}
          </select>
        </label>
        <label>
          Category
          <select
            name="category"
            value={selected.category || ''}
            disabled={loadingProject || !currentProject}
            onChange={(e) =>
              setSelected((existing) => ({
                project: existing.project,
                version: existing.version,
                category: e.target.value,
              }))
            }
          >
            <option value="">Default</option>
            {currentProject?.report_categories.map((category) => (
              <option key={category.id} value={category.id}>
                {category.name}
              </option>
            ))}
          </select>
        </label>
      </div>
      <div className="grid">
        <label>
          Mode
          <select
            name="mode"
            value={mode}
            onChange={(e) => updateMode(e.target.value)}
          >
            <option value="overview">Overview (Project info & measures)</option>
            <option value="measures">Measures (Simple report data)</option>
            <option value="report">
              Report (Full report data, very large!)
            </option>
            <option value="shield">Shield (Progress badge)</option>
            <option value="history">History (Historical report data)</option>
          </select>
        </label>
        <label>
          Format
          <select
            name="format"
            value={format}
            onChange={(e) => updateFormat(e.target.value)}
          >
            {formatsByMode[mode].map((format) => {
              switch (format) {
                case 'json':
                  return (
                    <option key={format} value={format}>
                      JSON
                    </option>
                  );
                case 'binpb':
                  return (
                    <option key={format} value={format}>
                      Protobuf
                    </option>
                  );
                case 'svg':
                  return (
                    <option key={format} value={format}>
                      SVG
                    </option>
                  );
                case 'png':
                  return (
                    <option key={format} value={format}>
                      PNG
                    </option>
                  );
              }
            })}
          </select>
        </label>
      </div>
      <div className="grid">
        <label>
          <input
            type="checkbox"
            name="useProjectId"
            checked={useProjectId}
            onChange={(e) => setUseProjectId(e.target.checked)}
          />
          Use project ID instead of repository name
        </label>
      </div>
      <hr />
      {modeOptions}
      {currentProject && <ApiPreview url={url.toString()} format={format} />}
    </div>
  );
};

const formatsByMode: Record<string, string[]> = {
  overview: ['json', 'svg', 'png'],
  measures: ['json', 'binpb'],
  report: ['json', 'binpb'],
  shield: ['svg', 'png', 'json'],
  history: ['json'],
};

const prettifyJson = (data: string): string => {
  let parsed: any;
  try {
    parsed = JSON.parse(data);
  } catch (error) {
    // Return the original data if parsing fails
    return data;
  }
  return JSON.stringify(parsed, null, 2);
};

const ApiPreview = ({
  url,
  format,
}: { url: string | null; format: string }) => {
  type ResponseData = {
    data: string;
    format: string;
  };
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [response, setResponse] = useState<ResponseData | null>(null);

  // biome-ignore lint/correctness/useExhaustiveDependencies:
  useEffect(() => {
    setResponse(null);
  }, [url]);

  const fetchData = async () => {
    if (!url) {
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const response = await fetchApiUrl(url);
      setResponse({ data: response, format });
    } catch (error) {
      console.error('Error fetching API URL:', error);
      setError('Failed to fetch data');
    } finally {
      setLoading(false);
    }
  };

  const stringified = useMemo(() => {
    if (!response) {
      return '';
    }
    const maxLength = 250000; // 250 KB
    let stringified: string;
    if (response.format === 'json') {
      stringified = prettifyJson(response.data);
    } else if (response.format === 'binpb') {
      stringified = `// Binary protobuf size ${formatBytes(response.data.length)}`;
    } else {
      stringified = response.data;
    }
    if (stringified.length > maxLength) {
      return `// Response was ${formatBytes(response.data.length)}, preview truncated to ${formatBytes(maxLength, 0)}\n${stringified.slice(0, maxLength)}`;
    }
    return stringified;
  }, [response]);

  if (!url) {
    return null;
  }

  let preview = null;
  if (response) {
    if (response.format === 'json' || response.format === 'binpb') {
      preview = (
        <CodeBlock
          value={stringified}
          originalValue={response.data}
          language="json"
        />
      );
    }
  }
  if (error) {
    preview = <CodeBlock value={error} language="plaintext" />;
  }

  if (format === 'svg' || format === 'png') {
    preview = <img src={url} alt="Preview" />;
  } else if (format === 'json' || format === 'binpb') {
    preview = (
      <>
        <div className="grid">
          <button
            className="secondary"
            onClick={fetchData}
            disabled={loading || (format !== 'json' && format !== 'binpb')}
          >
            Preview
          </button>
        </div>
        <br />
        {preview}
      </>
    );
  }

  return (
    <>
      <CodeBlock value={url.toString()} language="plaintext" />
      {preview}
    </>
  );
};

function formatBytes(bytes: number, decimals = 2): string {
  if (bytes === 0) return '0 B';

  const k = 1000;
  const dm = decimals < 0 ? 0 : decimals;
  const sizes = ['B', 'kB', 'MB', 'GB', 'TB', 'PB', 'EB', 'ZB', 'YB'];

  const sign = bytes < 0 ? '-' : '';
  const absoluteBytes = Math.abs(bytes);
  const i = Math.min(
    Math.floor(Math.log10(absoluteBytes) / Math.log10(k)),
    sizes.length - 1,
  );

  const value = absoluteBytes / k ** i;
  return `${sign}${value.toFixed(dm)} ${sizes[i]}`;
}

const rootNode = document.getElementById('root');
if (rootNode) {
  const root = createRoot(rootNode);
  root.render(
    <StrictMode>
      <ProjectForm />
    </StrictMode>,
  );
}
