import { CheckIcon, KeyIcon, ShieldCheckIcon } from "@primer/octicons-react";
import { useState, type FormEvent } from "react";
import QRCode from "react-qr-code";

import { isApiError } from "../api/client";
import { auth } from "../api/endpoints";
import {
  twoFactorDisableFormSchema,
  verifyCodeFormSchema,
  type TwoFactorSetup,
} from "../api/schemas";
import { validateForm, type FieldErrors } from "../lib/forms";
import { useAsync } from "../lib/useAsync";
import { CopyButton } from "./CopyButton";
import { Box, BoxBody, BoxHeader } from "./primitives/Box";
import { Button } from "./primitives/Button";
import { Flash } from "./primitives/Flash";
import { Label } from "./primitives/Label";
import { ErrorState, LoadingState } from "./primitives/StateViews";
import { TextInput } from "./primitives/TextInput";

/** `isApiError` first, then the caller's context-specific sentence. */
function describeError(error: unknown, fallback: string): string {
  return isApiError(error) ? error.message : fallback;
}

/**
 * Not-enabled setup runs in two stages: `scan` verifies the authenticator app,
 * `codes` shows the one-time backup codes and confirms activation. Activation
 * is only reachable from `codes`, so it can never happen without a successful
 * verify in this session.
 */
type SetupStage = "scan" | "codes";

function BackupCodes({
  codes,
  onDone,
}: {
  codes: string[];
  onDone?: () => void;
}) {
  return (
    <Flash variant="warning" title="Save your backup codes">
      <div className="space-y-2">
        <p>
          These codes are shown <strong>once</strong>. Each one signs you in if
          you lose access to your authenticator app. Store them somewhere safe -
          they cannot be retrieved again.
        </p>
        <ul className="grid grid-cols-2 gap-1.5 rounded-md border border-attention bg-canvas-default p-2 font-mono text-xs">
          {codes.map((backupCode) => (
            <li key={backupCode} className="select-all">
              {backupCode}
            </li>
          ))}
        </ul>
        <div className="flex flex-wrap items-center gap-2">
          <CopyButton value={codes.join("\n")} label="Copy codes" />
          {onDone ? (
            <Button
              size="sm"
              variant="primary"
              leadingIcon={<CheckIcon size={14} aria-hidden="true" />}
              onClick={onDone}
            >
              I have saved them
            </Button>
          ) : null}
        </div>
      </div>
    </Flash>
  );
}

export function TwoFactorPanel() {
  const state = useAsync(
    (signal) => auth.twoFactorStatus({ signal }),
    "two-factor",
  );

  const [actionError, setActionError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const [setup, setSetup] = useState<TwoFactorSetup | null>(null);
  const [stage, setStage] = useState<SetupStage | null>(null);
  const [startingSetup, setStartingSetup] = useState(false);
  const [verifyCode, setVerifyCode] = useState("");
  const [verifyErrors, setVerifyErrors] = useState<FieldErrors>({});
  const [verifying, setVerifying] = useState(false);
  const [enabling, setEnabling] = useState(false);

  const [regenerateOpen, setRegenerateOpen] = useState(false);
  const [regenCode, setRegenCode] = useState("");
  const [regenErrors, setRegenErrors] = useState<FieldErrors>({});
  const [regenerating, setRegenerating] = useState(false);
  const [newCodes, setNewCodes] = useState<string[] | null>(null);

  const [disableOpen, setDisableOpen] = useState(false);
  const [disablePassword, setDisablePassword] = useState("");
  const [disableCode, setDisableCode] = useState("");
  const [disableErrors, setDisableErrors] = useState<FieldErrors>({});
  const [disabling, setDisabling] = useState(false);

  const status = state.data;

  const startSetup = () => {
    setActionError(null);
    setNotice(null);
    setStartingSetup(true);
    void auth.twoFactorSetup().then(
      (result) => {
        setStartingSetup(false);
        setSetup(result);
        setStage("scan");
        setVerifyCode("");
        setVerifyErrors({});
      },
      (error: unknown) => {
        setStartingSetup(false);
        setActionError(
          describeError(
            error,
            "Two-factor authentication could not be started.",
          ),
        );
      },
    );
  };

  const cancelSetup = () => {
    setSetup(null);
    setStage(null);
    setVerifyCode("");
    setVerifyErrors({});
    setActionError(null);
  };

  const onVerify = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setActionError(null);
    setNotice(null);

    const validation = validateForm(verifyCodeFormSchema, { code: verifyCode });
    if (!validation.ok) {
      setVerifyErrors(validation.errors);
      return;
    }
    setVerifyErrors({});

    setVerifying(true);
    void auth.twoFactorVerify(validation.value.code).then(
      () => {
        setVerifying(false);
        setStage("codes");
      },
      (error: unknown) => {
        setVerifying(false);
        setActionError(
          describeError(error, "That code could not be verified. Try again."),
        );
      },
    );
  };

  const onEnable = () => {
    if (stage !== "codes" || !setup) return;

    setActionError(null);
    setNotice(null);
    setEnabling(true);
    void auth.twoFactorEnable().then(
      () => {
        setEnabling(false);
        setSetup(null);
        setStage(null);
        setVerifyCode("");
        setVerifyErrors({});
        setNotice("Two-factor authentication is now enabled.");
        state.reload();
      },
      (error: unknown) => {
        setEnabling(false);
        setActionError(
          describeError(
            error,
            "Two-factor authentication could not be enabled.",
          ),
        );
      },
    );
  };

  const closeRegenerate = () => {
    setRegenerateOpen(false);
    setRegenCode("");
    setRegenErrors({});
  };

  const onRegenerate = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setActionError(null);
    setNotice(null);

    const validation = validateForm(verifyCodeFormSchema, { code: regenCode });
    if (!validation.ok) {
      setRegenErrors(validation.errors);
      return;
    }
    setRegenErrors({});

    setRegenerating(true);
    void auth.twoFactorBackupCodes(validation.value.code).then(
      (result) => {
        setRegenerating(false);
        setRegenerateOpen(false);
        setRegenCode("");
        setNewCodes(result.backup_codes);
        setNotice("Your backup codes have been regenerated.");
        state.reload();
      },
      (error: unknown) => {
        setRegenerating(false);
        setActionError(
          describeError(error, "New backup codes could not be generated."),
        );
      },
    );
  };

  const closeDisable = () => {
    setDisableOpen(false);
    setDisablePassword("");
    setDisableCode("");
    setDisableErrors({});
  };

  const onDisable = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setActionError(null);
    setNotice(null);

    const validation = validateForm(twoFactorDisableFormSchema, {
      password: disablePassword,
      code: disableCode,
    });
    if (!validation.ok) {
      setDisableErrors(validation.errors);
      return;
    }
    setDisableErrors({});

    setDisabling(true);
    void auth
      .twoFactorDisable(validation.value.password, validation.value.code)
      .then(
        () => {
          setDisabling(false);
          setDisableOpen(false);
          setDisablePassword("");
          setDisableCode("");
          setNewCodes(null);
          setNotice("Two-factor authentication has been disabled.");
          state.reload();
        },
        (error: unknown) => {
          setDisabling(false);
          setActionError(
            describeError(
              error,
              "Two-factor authentication could not be disabled.",
            ),
          );
        },
      );
  };

  return (
    <Box>
      <BoxHeader>
        <span className="flex items-center gap-2 text-sm font-medium">
          <ShieldCheckIcon size={16} aria-hidden="true" />
          Two-factor authentication
        </span>
      </BoxHeader>
      <BoxBody className="space-y-3">
        {actionError ? (
          <Flash variant="danger" onDismiss={() => setActionError(null)}>
            {actionError}
          </Flash>
        ) : null}
        {notice ? (
          <Flash variant="success" onDismiss={() => setNotice(null)}>
            {notice}
          </Flash>
        ) : null}

        {state.loading && !status ? (
          <LoadingState label="Loading two-factor authentication…" />
        ) : null}
        {state.error ? (
          <ErrorState error={state.error} onRetry={state.reload} />
        ) : null}

        {status && !status.enabled && !setup ? (
          <div className="space-y-3">
            <p className="text-sm text-muted">
              Two-factor authentication adds a second step when you sign in.
              After your password, you enter a short-lived 6-digit code from an
              authenticator app such as 1Password, Authy or Google
              Authenticator. We&apos;ll also give you one-time backup codes in
              case you lose your device.
            </p>
            <Button
              variant="primary"
              onClick={startSetup}
              disabled={startingSetup}
            >
              {startingSetup ? "Starting…" : "Enable two-factor authentication"}
            </Button>
          </div>
        ) : null}

        {status && !status.enabled && setup && stage === "scan" ? (
          <div className="space-y-4">
            <p className="text-xs font-medium text-muted">Step 1 of 2</p>
            <div className="grid gap-4 sm:grid-cols-2">
              <div className="space-y-3">
                <div className="flex justify-center rounded-md border border-border bg-white p-4">
                  <QRCode
                    value={setup.otpauth_uri}
                    size={192}
                    title="Two-factor authentication QR code"
                  />
                </div>
                <div className="space-y-1">
                  <p className="text-xs text-muted">
                    Can&apos;t scan the code? Enter this secret manually in your
                    authenticator app.
                  </p>
                  <div className="flex flex-col gap-2 rounded-md border border-border bg-canvas-subtle p-2 sm:flex-row sm:items-center sm:justify-between">
                    <code className="flex items-center gap-2 break-all font-mono text-xs select-all">
                      <KeyIcon size={14} aria-hidden="true" />
                      {setup.secret}
                    </code>
                    <CopyButton value={setup.secret} label="Copy secret" />
                  </div>
                </div>
              </div>

              <form className="space-y-3" onSubmit={onVerify} noValidate>
                <p className="text-sm">
                  Scan the QR code with your authenticator app, then enter the
                  6-digit code it shows to confirm the app is set up.
                  You&apos;ll get your backup codes in the next step.
                </p>
                <TextInput
                  label="Verification code"
                  value={verifyCode}
                  onChange={(event) => setVerifyCode(event.target.value)}
                  error={verifyErrors.code}
                  hint="Enter the 6-digit code your authenticator app shows."
                  inputMode="numeric"
                  autoComplete="one-time-code"
                  maxLength={6}
                  required
                />
                <div className="flex flex-wrap gap-2">
                  <Button type="submit" variant="primary" disabled={verifying}>
                    {verifying ? "Verifying…" : "Verify code"}
                  </Button>
                  <Button onClick={cancelSetup} disabled={verifying}>
                    Cancel
                  </Button>
                </div>
              </form>
            </div>
          </div>
        ) : null}

        {status && !status.enabled && setup && stage === "codes" ? (
          <div className="space-y-3">
            <p className="text-xs font-medium text-muted">Step 2 of 2</p>
            <p className="text-sm">
              Save these backup codes somewhere safe, then confirm to turn on
              two-factor authentication.
            </p>
            <BackupCodes codes={setup.backup_codes} onDone={onEnable} />
            <div className="flex flex-wrap gap-2">
              <Button onClick={cancelSetup} disabled={enabling}>
                Cancel
              </Button>
            </div>
          </div>
        ) : null}

        {status && status.enabled ? (
          <div className="space-y-3">
            <div className="flex flex-wrap items-center gap-2">
              <Label variant="success">Enabled</Label>
              <span className="text-sm text-muted">
                {status.backup_codes_remaining} backup{" "}
                {status.backup_codes_remaining === 1 ? "code" : "codes"}{" "}
                remaining.
              </span>
              {status.backup_codes_remaining === 0 ? (
                <Label variant="attention">No backup codes left</Label>
              ) : null}
            </div>

            {newCodes ? (
              <BackupCodes codes={newCodes} onDone={() => setNewCodes(null)} />
            ) : null}

            {regenerateOpen ? (
              <form
                className="space-y-3 rounded-md border border-border bg-canvas-subtle p-3"
                onSubmit={onRegenerate}
                noValidate
              >
                <p className="text-sm">
                  Enter a code from your authenticator app to generate a fresh
                  set of backup codes. Your current codes stop working
                  immediately.
                </p>
                <TextInput
                  label="Authentication code"
                  value={regenCode}
                  onChange={(event) => setRegenCode(event.target.value)}
                  error={regenErrors.code}
                  inputMode="numeric"
                  autoComplete="one-time-code"
                  maxLength={6}
                  required
                />
                <div className="flex flex-wrap gap-2">
                  <Button
                    type="submit"
                    variant="primary"
                    disabled={regenerating}
                  >
                    {regenerating ? "Generating…" : "Regenerate codes"}
                  </Button>
                  <Button onClick={closeRegenerate} disabled={regenerating}>
                    Cancel
                  </Button>
                </div>
              </form>
            ) : (
              <Button
                onClick={() => {
                  setRegenerateOpen(true);
                  setRegenCode("");
                  setRegenErrors({});
                }}
              >
                Regenerate backup codes
              </Button>
            )}

            {disableOpen ? (
              <form
                className="space-y-3 rounded-md border border-danger p-3"
                onSubmit={onDisable}
                noValidate
              >
                <p className="text-sm">
                  Disabling two-factor authentication removes the second sign-in
                  step and invalidates your backup codes. Enter your password
                  and a current code to confirm.
                </p>
                <div className="grid gap-3 sm:grid-cols-2">
                  <TextInput
                    label="Password"
                    type="password"
                    value={disablePassword}
                    onChange={(event) => setDisablePassword(event.target.value)}
                    error={disableErrors.password}
                    autoComplete="current-password"
                    required
                  />
                  <TextInput
                    label="Authentication or backup code"
                    value={disableCode}
                    onChange={(event) => setDisableCode(event.target.value)}
                    error={disableErrors.code}
                    inputMode="numeric"
                    autoComplete="one-time-code"
                    required
                  />
                </div>
                <div className="flex flex-wrap gap-2">
                  <Button type="submit" variant="danger" disabled={disabling}>
                    {disabling
                      ? "Disabling…"
                      : "Disable two-factor authentication"}
                  </Button>
                  <Button onClick={closeDisable} disabled={disabling}>
                    Cancel
                  </Button>
                </div>
              </form>
            ) : (
              <Button
                variant="danger"
                onClick={() => {
                  setDisableOpen(true);
                  setDisablePassword("");
                  setDisableCode("");
                  setDisableErrors({});
                }}
              >
                Disable two-factor authentication
              </Button>
            )}
          </div>
        ) : null}
      </BoxBody>
    </Box>
  );
}
