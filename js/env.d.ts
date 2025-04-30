/// <reference types="@rsbuild/core/types" />

type Unit = {
  name: string;
  fuzzy_match_percent: number;
  total_code: number;
  color: string;
  x: number;
  y: number;
  w: number;
  h: number;
};

type Measures = {
  fuzzy_match_percent: number;
  total_code: number;
  matched_code: number;
  matched_code_percent: number;
  total_data: number;
  matched_data: number;
  matched_data_percent: number;
  total_functions: number;
  matched_functions: number;
  matched_functions_percent: number;
  complete_code: number;
  complete_code_percent: number;
  complete_data: number;
  complete_data_percent: number;
  total_units: number;
  complete_units: number;
};

type ReportHistoryEntry = {
  timestamp: string;
  commit_sha: string;
  measures: Measures;
};

interface Window {
  drawTreemap: (id: string, clickable: boolean, units: Unit[]) => void;
  renderChart: (id: string, data: ReportHistoryEntry[]) => void;
}
