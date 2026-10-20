import React from "react";
import { ErrorState } from "@symphony/ui";

interface Props {
  title: string;
  detail: string;
  retryLabel: string;
  /** Changing this resets the boundary (e.g. the route path). */
  resetKey?: string;
  children: React.ReactNode;
}

interface State {
  error: Error | null;
}

/** Route-level error boundary: a crashing view never takes down the shell. */
export class ErrorBoundary extends React.Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("[symphony] view crashed", error, info.componentStack);
  }

  componentDidUpdate(prev: Props) {
    if (prev.resetKey !== this.props.resetKey && this.state.error) this.setState({ error: null });
  }

  render() {
    if (this.state.error) {
      return (
        <div className="p-4">
          <ErrorState
            title={this.props.title}
            message={`${this.props.detail} (${this.state.error.message})`}
            retryLabel={this.props.retryLabel}
            onRetry={() => this.setState({ error: null })}
          />
        </div>
      );
    }
    return this.props.children;
  }
}
