--------------------------- MODULE CredentialRouting ---------------------------
(***************************************************************************
0044/0048 credential contract: a declaration's base URL and repo build env
are not credential authority. Hosts are already-canonical symbolic values;
Rust URL parsing, token loading, ports, TLS verification and keychains are NOT
proved by this model. Redirect forwarding is conservatively overapproximated:
the real HTTP client drops auth on every redirect, while the model permits
same-host HTTPS forwarding. The sandbox cannot constrain a trusted module
running arbitrary credentialed shell code.
***************************************************************************)
EXTENDS Naturals
CONSTANTS BindFromBaseUrl, ForwardAcrossRedirect, RepoMayRebindAudience
Hosts == {"github.com", "api.github.com", "enterprise-one", "enterprise-two"}
PublicHosts == {"github.com", "api.github.com"}
EnterpriseHosts == Hosts \ PublicHosts
Tokens == {"none", "public", "enterprise"}
VARIABLES declaredHost, operatorEnterpriseHost, boundEnterpriseHost,
          publicPresent, enterprisePresent, requestHost, redirectedHost,
          initialScheme, redirectedScheme, phase, sentToken, sentHost, viaRedirect
variables == <<declaredHost, operatorEnterpriseHost, boundEnterpriseHost,
               publicPresent, enterprisePresent, requestHost, redirectedHost,
               initialScheme, redirectedScheme, phase, sentToken, sentHost,
               viaRedirect>>

SelectToken(host) ==
    IF host \in PublicHosts THEN IF publicPresent THEN "public" ELSE "none"
    ELSE IF enterprisePresent /\ (host = boundEnterpriseHost
            \/ (BindFromBaseUrl /\ host = declaredHost)) THEN "enterprise"
    ELSE "none"

Init ==
    /\ declaredHost \in EnterpriseHosts
    /\ operatorEnterpriseHost \in EnterpriseHosts \union {"unbound"}
    /\ boundEnterpriseHost = IF RepoMayRebindAudience THEN declaredHost
                              ELSE operatorEnterpriseHost
    /\ publicPresent \in BOOLEAN /\ enterprisePresent \in BOOLEAN
    /\ requestHost \in Hosts /\ redirectedHost \in Hosts
    /\ initialScheme \in {"http", "https"}
    /\ redirectedScheme \in {"http", "https"}
    /\ phase = "initial" /\ sentToken = "none" /\ sentHost = requestHost
    /\ viaRedirect = FALSE

Send ==
    /\ phase = "initial"
    /\ sentToken' = IF initialScheme = "https" THEN SelectToken(requestHost)
                     ELSE "none"
    /\ sentHost' = requestHost
    /\ phase' = "sent"
    /\ UNCHANGED <<declaredHost, operatorEnterpriseHost, boundEnterpriseHost,
                   publicPresent, enterprisePresent, requestHost, redirectedHost,
                   initialScheme, redirectedScheme, viaRedirect>>

Redirect ==
    /\ phase = "sent"
    /\ LET allowed == redirectedHost = requestHost
                       /\ ~(initialScheme = "https" /\ redirectedScheme = "http") IN
       sentToken' = IF allowed \/ ForwardAcrossRedirect THEN sentToken ELSE "none"
    /\ sentHost' = redirectedHost /\ viaRedirect' = TRUE /\ phase' = "done"
    /\ UNCHANGED <<declaredHost, operatorEnterpriseHost, boundEnterpriseHost,
                   publicPresent, enterprisePresent, requestHost, redirectedHost,
                   initialScheme, redirectedScheme>>

Next == Send \/ Redirect
Spec == Init /\ [][Next]_variables /\ WF_variables(Next)
TypeOK == /\ phase \in {"initial", "sent", "done"}
          /\ operatorEnterpriseHost \in EnterpriseHosts \union {"unbound"}
          /\ boundEnterpriseHost \in EnterpriseHosts \union {"unbound"}
          /\ sentHost \in Hosts /\ sentToken \in Tokens /\ viaRedirect \in BOOLEAN
TokensStayBound ==
    /\ (sentToken = "public" => sentHost \in PublicHosts)
    /\ (sentToken = "enterprise" => sentHost = operatorEnterpriseHost)
NoRedirectDisclosure == viaRedirect /\ (redirectedHost # requestHost
    \/ (initialScheme = "https" /\ redirectedScheme = "http")) => sentToken = "none"
NoCleartextCredential == sentToken # "none" =>
    IF viaRedirect THEN redirectedScheme = "https" ELSE initialScheme = "https"
EventuallyRouted == <>(phase = "done")
=============================================================================
