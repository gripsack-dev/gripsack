--------------------------- MODULE CredentialRouting ---------------------------
(***************************************************************************
0044 credential contract: a declaration's base URL is not credential authority.
Hosts are already-canonical symbolic values; Rust URL parsing, token loading,
ports, TLS verification and keychains are NOT proved by this model. It preserves
current host binding and same-host/no-downgrade redirect policy, not a sandbox
against a trusted module that can execute arbitrary credentialed shell code.
***************************************************************************)
EXTENDS Naturals
CONSTANTS BindFromBaseUrl, ForwardAcrossRedirect
Hosts == {"github.com", "api.github.com", "enterprise-one", "enterprise-two"}
PublicHosts == {"github.com", "api.github.com"}
EnterpriseHosts == Hosts \ PublicHosts
Tokens == {"none", "public", "enterprise"}
VARIABLES declaredHost, boundEnterpriseHost, publicPresent, enterprisePresent,
          requestHost, redirectedHost, initialScheme, redirectedScheme,
          phase, sentToken, sentHost, viaRedirect
variables == <<declaredHost, boundEnterpriseHost, publicPresent, enterprisePresent,
               requestHost, redirectedHost, initialScheme, redirectedScheme,
               phase, sentToken, sentHost, viaRedirect>>

SelectToken(host) ==
    IF host \in PublicHosts THEN IF publicPresent THEN "public" ELSE "none"
    ELSE IF enterprisePresent /\ (host = boundEnterpriseHost
            \/ (BindFromBaseUrl /\ host = declaredHost)) THEN "enterprise"
    ELSE "none"

Init ==
    /\ declaredHost \in EnterpriseHosts
    /\ boundEnterpriseHost \in EnterpriseHosts \union {"unbound"}
    /\ publicPresent \in BOOLEAN /\ enterprisePresent \in BOOLEAN
    /\ requestHost \in Hosts /\ redirectedHost \in Hosts
    /\ initialScheme \in {"http", "https"}
    /\ redirectedScheme \in {"http", "https"}
    /\ phase = "initial" /\ sentToken = "none" /\ sentHost = requestHost
    /\ viaRedirect = FALSE

Send ==
    /\ phase = "initial"
    /\ sentToken' = SelectToken(requestHost) /\ sentHost' = requestHost
    /\ phase' = "sent"
    /\ UNCHANGED <<declaredHost, boundEnterpriseHost, publicPresent, enterprisePresent,
                   requestHost, redirectedHost, initialScheme, redirectedScheme, viaRedirect>>

Redirect ==
    /\ phase = "sent"
    /\ LET allowed == redirectedHost = requestHost
                       /\ ~(initialScheme = "https" /\ redirectedScheme = "http") IN
       sentToken' = IF allowed \/ ForwardAcrossRedirect THEN sentToken ELSE "none"
    /\ sentHost' = redirectedHost /\ viaRedirect' = TRUE /\ phase' = "done"
    /\ UNCHANGED <<declaredHost, boundEnterpriseHost, publicPresent, enterprisePresent,
                   requestHost, redirectedHost, initialScheme, redirectedScheme>>

Next == Send \/ Redirect
Spec == Init /\ [][Next]_variables /\ WF_variables(Next)
TypeOK == /\ phase \in {"initial", "sent", "done"}
          /\ sentHost \in Hosts /\ sentToken \in Tokens /\ viaRedirect \in BOOLEAN
TokensStayBound ==
    /\ (sentToken = "public" => sentHost \in PublicHosts)
    /\ (sentToken = "enterprise" => sentHost = boundEnterpriseHost)
NoRedirectDisclosure == viaRedirect /\ (redirectedHost # requestHost
    \/ (initialScheme = "https" /\ redirectedScheme = "http")) => sentToken = "none"
EventuallyRouted == <>(phase = "done")
=============================================================================
