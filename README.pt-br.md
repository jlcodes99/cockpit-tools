# Cockpit Tools

[English](README.en.md)  · Portuguese (BR) · [简体中文](README.md)

[![GitHub stars](https://img.shields.io/github/stars/jlcodes99/cockpit-tools?style=flat&color=gold)](https://github.com/jlcodes99/cockpit-tools)
[![GitHub downloads](https://img.shields.io/github/downloads/jlcodes99/cockpit-tools/total?style=flat&color=blue)](https://github.com/jlcodes99/cockpit-tools/releases)
[![GitHub release](https://img.shields.io/github/v/release/jlcodes99/cockpit-tools?style=flat)](https://github.com/jlcodes99/cockpit-tools/releases)
[![GitHub issues](https://img.shields.io/github/issues/jlcodes99/cockpit-tools)](https://github.com/jlcodes99/cockpit-tools/issues)

Uma **ferramenta universal de gerenciamento de contas para IDEs de IA**, atualmente compatível com **Antigravity IDE**, **Codex**, **GitHub Copilot**, **Windsurf**, **Kiro**, **Cursor**, **Grok CLI**, **CodeBuddy**, **CodeBuddy CN**, **Qoder**, **Trae**, **TRAE SOLO**, **Trae CN**, **TRAE SOLO CN**, **Zed** e **ZCode**, com fluxos de trabalho paralelos em múltiplas instâncias.

> Projetada para ajudar os usuários a gerenciar com eficiência múltiplas contas de IDE com IA, esta ferramenta oferece suporte à troca com um clique, monitoramento de cota, tarefas de ativação e execuções paralelas em múltiplas instâncias, ajudando você a utilizar totalmente os recursos de diferentes contas.

**Recursos**: Alternância com um clique · Gerenciamento de múltiplas contas · Múltiplas instâncias · Monitoramento de cotas · Tarefas de ativação · Integração de plugins · Gerenciamento do GitHub Copilot · Gerenciamento do Windsurf · Gerenciamento do Kiro · Gerenciamento do Cursor · Gerenciamento do Grok CLI · Gerenciamento do CodeBuddy · Gerenciamento do CodeBuddy CN · Gerenciamento do Qoder · Gerenciamento do pacote Trae · Gerenciamento do Zed · Gerenciamento do ZCode

**Idiomas**: Suporta 18 idiomas

🇺🇸 English · 🇨🇳 简体中文 · 繁體中文 · 🇯🇵 日本語 · 🇩🇪 Deutsch · 🇪🇸 Español · 🇫🇷 Français · 🇮🇹 Italiano · 🇰🇷 한국어 · 🇧🇷 Português · 🇷🇺 Русский · 🇹🇷 Türkçe · 🇵🇱 Polski · 🇨🇿 Čeština · 🇸🇦 العربية · 🇻🇳 Tiếng Việt · 🇮🇩 Bahasa Indonesia

**Plataformas oficialmente suportadas**: macOS, Windows e Linux.

---

## Visão Geral dos Recursos

### 1. Painel

Um painel visual totalmente novo que oferece uma visão geral do status em um único lugar:

- **Suporte a dezesseis Plataformas**: Exibe simultaneamente o status das contas do Antigravity IDE, Codex, GitHub Copilot, Windsurf, Kiro, Cursor, Grok CLI, CodeBuddy, CodeBuddy CN, Qoder, Trae, TRAE SOLO, Trae CN, TRAE SOLO CN, Zed e ZCode
- **Monitoramento de Cotas**: Visualização em tempo real das cotas restantes e dos horários de redefinição para cada modelo
- **Ações Rápidas**: Atualização com um clique, ativação com um clique
- **Progresso Visual**: Barras de progresso intuitivas mostrando o consumo de cotas

> ![Visão geral do painel](docs/images/dashboard_overview.png)

### 2. Gerenciamento de Contas do Antigravity IDE

- **Troca com Um Clique**: Altere instantaneamente a conta ativa no momento sem login/logout manual
- **Múltiplos Métodos de Importação**: OAuth, Token de Atualização (Refresh Token), Sincronização de Plugin
- **Tarefas de Ativação**: Agende ativações de modelos de IA para acionar ciclos de redefinição de cota com antecedência

> ![Contas do Antigravity IDE](docs/images/antigravity_list.png)
>
> *(Tarefas de Despertar)*
> ![Tarefas de Despertar](docs/images/wakeup_detail.png)

#### 2.1 Múltiplas Instâncias do Antigravity IDE

Execute várias instâncias do Antigravity IDE em paralelo com contas diferentes. Por exemplo, abra duas instâncias do Antigravity IDE, vincule contas diferentes e gerencie projetos diferentes de forma independente.

- **Contas Isoladas**: Cada instância vincula uma conta diferente e executa de forma independente
- **Projetos Paralelos**: Execute várias tarefas/projetos ao mesmo tempo
- **Isolamento de Argumentos**: Diretório de instância personalizado e argumentos de inicialização

> ![Instâncias do IDE Antigravity](docs/images/antigravity_instances.png)

### 3. Gerenciamento de Conta do Codex

- **Suporte Dedicado**: Experiência de gerenciamento de conta otimizada para o Codex
- **Exibição de Cotas**: Exibição clara do status das cotas Horária e Semanal
- **Reconhecimento de Plano**: Identifica automaticamente os tipos de plano da conta (Basic, Plus, Team, etc.)

> ![Codex Accounts](docs/images/codex_list.png)

#### 3.1 Codex Multi-Instance

O Codex também suporta o uso de múltiplas instâncias em paralelo. Por exemplo, abra duas instâncias do Codex, vincule contas diferentes e execute projetos diferentes de forma independente.

- **Contas Isoladas**: Cada instância vincula uma conta diferente e é executada de forma independente.
- **Projetos Paralelos**: Execute várias tarefas/projetos simultaneamente.
- **Isolamento de Argumentos**: Diretório de instância e argumentos de inicialização personalizados.

- **Isolated Accounts**: Each instance binds a different account and runs independently
- **Parallel Projects**: Run multiple tasks/projects at the same time
- **Argument Isolation**: Custom instance directory and launch arguments

> ![Instâncias do Codex](docs/images/codex_instances.png)

### 4. Gerenciamento de Contas do GitHub Copilot

- **Importação de Contas**: Importação de OAuth, Token/JSON
- **Visualização de Cotas**: Sugestões embutidas / Uso de mensagens de chat e tempo de redefinição
- **Reconhecimento de Planos**: Detecção automática dos planos Gratuito / Individual / Pro / Business / Enterprise
- **Operações em Lote**: Tags e ações em massa

#### 4.1 GitHub Copilot Multi-Instâncias

Gerencie instâncias do VS Code Copilot com perfis isolados e controles de ciclo de vida.

- **Perfis Isolados**: Cada instância usa seu próprio diretório de dados do usuário
- **Ciclo de Vida Rápido**: Iniciar/parar/forçar parada de instâncias
- **Controle de Janelas**: Abrir janelas de instâncias e fechar todas as instâncias

### 5. Gerenciamento de Contas do Windsurf

- **Importação de Contas**: Importação via OAuth, Token/JSON e importação local
- **Visualização de Cotas**: Exibe o Plano, os créditos de prompts do usuário, os créditos de prompts de complementos e informações do ciclo
- **Operações em Lote**: Tags e ações em massa
- **Injeção de Troca de Conta**: Suporta a injeção e inicialização do Windsurf após a troca de conta

#### 5.1 Windsurf Multi-Instâncias

Gerencie instâncias do Windsurf com perfis isolados e controles de ciclo de vida.

- **Perfis Isolados**: Cada instância utiliza seu próprio diretório de dados do usuário
- **Ciclo de Vida Rápido**: Inicie/pare/force a parada de instâncias
- **Controle de Janelas**: Abra janelas de instâncias e feche todas as instâncias

### 6. Gerenciamento de Contas Kiro

- **Importação de Contas**: Importação via OAuth, Token/JSON e importação local
- **Visualização de Cotas**: Exibe o Plano, os créditos de solicitação do usuário, os créditos de solicitação de complementos e informações do ciclo
- **Operações em Lote**: Tags e ações em massa
- **Injeção de Troca de Conta**: Suporta a injeção e inicialização do Kiro após a troca de conta

#### 6.1 Kiro Multi-Instância

Gerencie instâncias do Kiro com perfis isolados e controles de ciclo de vida.

- **Perfis Isolados**: Cada instância utiliza seu próprio diretório de dados do usuário.
- **Ciclo de Vida Rápido**: Inicie, pare e force a parada de instâncias.
- **Controle de Janelas**: Abra janelas de instâncias e feche todas as instâncias.

### 7. Gerenciamento de Contas do Cursor

- **Importação de Contas**: Importação via OAuth, Token/JSON e importação local
- **Visualização de Cotas**: Exibe o uso total, uso automático + Composer, uso da API, uso sob demanda e informações de ciclo
- **Operações em Lote**: Tags e ações em massa
- **Injeção de Troca de Conta**: Suporta a injeção e inicialização do Cursor após a troca de conta

#### 7.1 Cursor Multi-Instância

Gerencie instâncias do Cursor com perfis isolados e controles de ciclo de vida.

- **Perfis Isolados**: Cada instância utiliza seu próprio diretório de dados do usuário.
- **Ciclo de Vida Rápido**: Inicie, pare e force a parada de instâncias.
- **Controle de Janelas**: Abra janelas de instâncias e feche todas as instâncias.


### 8. Gerenciamento de Conta Grok CLI

- **Autorização OAuth**: Suporta o fluxo de dispositivo OIDC oficial da xAI e salva a conta após a conclusão da verificação no navegador
- **Chaves de API e Endpoints de Terceiros**: Suporta chaves de API oficiais da xAI e também `Base URL` e IDs de modelo compatíveis com OpenAI de terceiros; a configuração é gravada em um `config.toml` específico da conta, enquanto a chave é injetada apenas no processo da CLI correspondente na inicialização
- **Importação e Exportação com Omissão**: Importa credenciais oficiais do `~/.grok/auth.json` padrão ou de um JSON fornecido; as exportações da página de contas e os backups genéricos omitem os tokens de acesso/atualização, não conseguem restaurar um login e exigem uma importação separada do `auth.json` oficial ao migrar
- **Troca Real de Conta**: Grava a conta selecionada no `~/.grok/auth.json` padrão no formato de registro oficial do Grok CLI, preservando outros escopos de registro do arquivo
- **Cota e Plano**: Consulta os endpoints oficiais de faturamento/usuário/assinaturas, exibe ciclo, uso, cotas de produto e o valor bruto do plano, e registra o acesso ao Grok Code
- **Manutenção de Token**: Suporta atualização automática do token de acesso, rotação do token de atualização e alertas de cota

#### 8.1 Grok CLI Multi-Instância

A instância padrão do Grok CLI normalmente usa o diretório oficial `~/.grok` diretamente e inicia sem definir `GROK_HOME`; as instâncias gerenciadas usam diretórios separados. As contas com chave de API, incluindo endpoints de terceiros, sempre iniciam com um `GROK_HOME` específico da conta, para que uma sessão OAuth oficial não possa ter precedência sobre as credenciais.

- **Vinculação de Conta**: A instância padrão pode seguir a conta atual, enquanto cada instância gerenciada pode vincular uma conta diferente
- **Isolamento de Tempo de Execução**: As instâncias gerenciadas mantêm seus `auth.json`, `config.toml`, diretórios de trabalho e argumentos de inicialização separados
- **Ciclo de Vida no Terminal**: Gera ou executa comandos de inicialização no terminal, para instâncias e fecha todas as instâncias
- **Proteção de Diretório**: Instâncias não padrão ficam restritas à raiz gerenciada padrão e são movidas para a lixeira ao serem excluídas; caminhos externos de configurações legadas são apenas cancelados e nunca gravados ou excluídos

### 9. Gerenciamento de conta CodeBuddy

- **Importação de Contas**: Importação via OAuth e Token/JSON
- **Visualização de Cotas**: Consulta de cotas, detalhes do ciclo e exibição de créditos extras
- **Operações em Lote**: Tags e ações em massa
- **Injeção de Troca de Conta**: Suporta a injeção e inicialização do CodeBuddy após a troca de conta

#### 9.1 CodeBuddy Multi-Instância

Gerencie instâncias do CodeBuddy com perfis isolados e controles de ciclo de vida.

- **Perfis Isolados**: Cada instância utiliza seu próprio diretório de dados do usuário.
- **Ciclo de Vida Rápido**: Inicie, pare e force a parada de instâncias.
- **Controle de Janelas**: Abra janelas de instâncias e feche todas as instâncias.

### 10. Gerenciamento de Contas do CodeBuddy CN

- **Importação de Conta**: suporta importação de OAuth, Token/JSON e cliente local
- **Visualização de Cota**: exibe o plano e o status de uso, com um atalho para abrir informações detalhadas de cota na página oficial
- **Operações em Lote**: suporta tags e ações em massa
- **Injeção de Troca de Conta**: suporta a gravação do estado de autenticação local de volta e a inicialização do CodeBuddy CN após a troca de conta

#### 10.1 CodeBuddy CN Multi-Instância

Gerencie instâncias do CodeBuddy CN com perfis isolados e controles de ciclo de vida.

- **Perfis Isolados**: cada instância utiliza seu próprio diretório de dados do usuário.
- **Ciclo de Vida Rápido**: inicie, pare e force a parada de instâncias.
- **Controle de Janelas**: abra janelas de instâncias e feche todas as instâncias.

### 11. Gerenciamento de Contas do Qoder

- **Importação de Contas**: suporta importação local e importação JSON
- **Visualização de Cotas**: mostra o uso de créditos, créditos restantes e valores brutos do plano
- **Operações em Lote**: suporta tags, filtros, exportação e exclusão/atualização em lote
- **Injeção de Troca de Conta**: suporta a injeção e inicialização do Qoder após a troca de conta

#### 11.1 Qoder Multi-Instância

Gerencie instâncias do Qoder com perfis isolados e controles de ciclo de vida.

- **Perfis Isolados**: cada instância utiliza seu próprio diretório de dados do usuário.
- **Ciclo de Vida Rápido**: inicie, pare e force a parada de instâncias.
- **Controle de Janelas**: abra janelas de instâncias e feche todas as instâncias.

### 12. Gerenciamento de Contas Trae

- **Importação de Contas**: suporta importação local e importação JSON
- **Visualização de Cotas**: mostra os valores brutos do plano, USD gasto/orçamento total e tempo de reinicialização
- **Operações em Lote**: suporta tags, filtros, exportação e exclusão/atualização em lote
- **Pacote Trae**: suporta importação local e injeção de troca para os clientes padrão do Trae, TRAE SOLO, Trae CN e TRAE SOLO CN; eles são agrupados sob o Trae por padrão
- **Injeção de Troca de Conta**: suporta a gravação do estado de autenticação local e a inicialização do Trae após a troca de conta

#### 12.1 Trae Multi-Instância

Gerencie instâncias do Trae com perfis isolados e controles de ciclo de vida.

- **Perfis Isolados**: cada instância utiliza seu próprio diretório de dados do usuário.
- **Ciclo de Vida Rápido**: inicie, pare e force a parada de instâncias.
- **Controle de Janelas**: abra janelas de instâncias e feche todas as instâncias.

### 13. Gerenciamento de Contas Zed

- **Importação de Conta**: Suporta login OAuth oficial, importação JSON e importação do estado atual de login local.
- **Visualização de Uso**: Exibe o status da assinatura, permite editar previsões, gastos com tokens, limite de gastos e o fim do período de faturamento.
- **Operações em Lote**: Suporta tags, filtros, exportação e exclusão/atualização em lote.
- **Injeção de Switch**: Aplica a conta selecionada de volta ao cliente Zed oficial usando as regras de persistência local reais do cliente e reinicia o cliente quando necessário.

### 14. Gerenciamento de Conta ZCode

- **Login Oficial**: Com o ZCode fechado, conclua o OAuth Z.ai ou BigModel na janela de autorização integrada do Cockpit; ele captura o callback oficial `zcode://` diretamente e salva a conta
- **Importação e Exportação**: Lê credenciais locais criptografadas de `~/.zcode/v2/credentials.json`, importa ou exporta JSON e faz backup das contas
- **Visualização de Cota**: Consulta planos de assinatura e cotas por modelo, preservando os valores brutos do plano
- **Operações em Lote**: Tags, pesquisa, filtros de plano, exportação e exclusão/atualização em lote
- **Troca Real de Conta**: Criptografa e grava a conta selecionada de volta usando o formato oficial de credenciais do ZCode

#### 14.1 ZCode Multi-Instância

Gerencie instâncias do ZCode com dados de usuário, de sessão e de dados do ZCode do Electron separados.

- **Vinculação de Conta**: Vincule uma conta diferente a cada instância ou siga a conta atual
- **Tempo de Execução Isolado**: As credenciais e os dados do aplicativo de cada instância permanecem separados
- **Controles de Ciclo de Vida**: Inicie, pare, foque e feche todas as instâncias gerenciadas

### 15. Configurações Gerais

- **Configurações Personalizadas**: Troca de tema, configurações de idioma, intervalo de atualização automática
- **Controles da Plataforma**: Configurações centralizadas de caminho de inicialização e alerta de cota do Grok CLI/CodeBuddy CN/Qoder/pacote Trae/Zed/ZCode

> ![Configurações](docs/images/settings_page.png)

---

## Segurança e Privacidade (em linguagem simples)

Estas são as perguntas de segurança mais comuns respondidas diretamente:

- **Esta é uma ferramenta para desktop local**: ela não requer uma conta de nuvem separada para este projeto e não depende do armazenamento em nuvem hospedado pelo projeto.
- **Os dados são armazenados principalmente em sua máquina**:
  - `~/.antigravity_cockpit`: Contas do Antigravity IDE, configurações, status do WebSocket, etc.
  - `~/.codex`: Arquivo `auth.json` de login atual do Codex
  - `~/.grok`: a instância padrão oficial do Grok CLI e o `auth.json` do login atual
  - `~/.zcode/v2`: credenciais criptografadas do ZCode para o login oficial atual e o cache de cota
  - Pasta de dados do aplicativo local em `com.antigravity.cockpit-tools`: Dados de múltiplas contas do Codex / GitHub Copilot / Windsurf / Kiro / Cursor / Grok CLI / CodeBuddy / CodeBuddy CN / Qoder / pacote Trae / Zed / ZCode, etc.; detalhes de conta, perfis gerenciados e configurações de instância do Grok CLI também são armazenados aqui
- **As credenciais do Grok CLI não são criptografadas**: os tokens de acesso e de atualização são armazenados localmente como JSON em texto puro e dependem principalmente do isolamento de conta do sistema operacional e das permissões de arquivos locais. Em sistemas Unix, os diretórios de credenciais são definidos como `0700` e os arquivos de credenciais como `0600`. As exportações com omissão não contêm tokens e não servem como backups de login.
- **O WebSocket é somente local por padrão**: vincula-se a `127.0.0.1`, porta padrão `19528`; você pode desativá-lo ou alterar a porta em Configurações.
- **Quando ocorre acesso à rede**: login OAuth, atualização de token, obtenção de cota, verificações de atualização e outras solicitações oficiais da API.
**Solicitações de permissão de privacidade do macOS**: após iniciar o Codex/agente a partir do Cockpit Tools, se um comando do shell executado pelo agente acessar pastas protegidas, como Área de Trabalho, Documentos, Downloads ou Fotos, o macOS poderá exibir a solicitação como "O Cockpit Tools gostaria de acessar...". Isso ocorre porque esses comandos são processos filhos iniciados pelo Cockpit Tools, portanto, o macOS atribui a solicitação ao aplicativo host; isso não significa, por si só, que o processo principal do Cockpit Tools esteja ativamente verificando essas pastas. Conceda acesso somente se você confiar na tarefa atual do agente e nos comandos que ela executará. Em caso de dúvida, negue a solicitação ou execute o projeto a partir de um diretório de trabalho normal primeiro.
- **Dicas práticas de segurança**:
  1. Se você não precisa da integração de plugins, desative o WebSocket.
  2. Não compartilhe seu diretório de usuários completo diretamente; oculte os arquivos de token antes de fazer backup/compartilhar.
  3. Em computadores compartilhados/públicos, remova as contas e feche o aplicativo após o uso.
  
## Guia de Configurações (Ideal para Iniciantes)

Se você deseja uma configuração estável com ajustes mínimos, siga os valores "Recomendados".

### Configurações Gerais

| Setting | What it does (simple) | Recommended | When to change |
| --- | --- | --- | --- |
| Display Language | Changes UI language | Your native/comfortable language | Only if current language is hard to read |
| Theme | Light/dark appearance | System | Use dark mode for long night sessions |
| Window Close Behavior | What happens when clicking close | Ask every time | Choose "Minimize to tray" if you want background running |
| Antigravity IDE Auto Refresh | Periodically updates Antigravity IDE quota | 5-10 minutes | Use 2 minutes if you need near real-time updates |
| Codex Auto Refresh | Periodically updates Codex quota | 5-10 minutes | Same as above |
| GitHub Copilot Auto Refresh | Periodically updates GitHub Copilot quota | 5-10 minutes | Same as above |
| Windsurf Auto Refresh | Periodically updates Windsurf quota | 5-10 minutes | Same as above |
| Kiro Auto Refresh | Periodically updates Kiro quota | 5-10 minutes | Same as above |
| Cursor Auto Refresh | Periodically updates Cursor quota | 5-10 minutes | Same as above |
| Grok CLI Auto Refresh | Periodically refreshes tokens and updates quota | 5-10 minutes | Same as above |
| CodeBuddy Auto Refresh | Periodically updates CodeBuddy quota | 5-10 minutes | Same as above |
| CodeBuddy CN Auto Refresh | Periodically updates CodeBuddy CN quota | 5-10 minutes | Same as above |
| Qoder Auto Refresh | Periodically updates Qoder quota | 5-10 minutes | Same as above |
| Trae Auto Refresh | Periodically updates Trae suite account quota | 5-10 minutes | Same as above |
| Zed Auto Refresh | Periodically updates Zed quota | 5-10 minutes | Same as above |
| Data Directory | Where account/config files are stored | Keep default | Only for troubleshooting or backups |
| Antigravity IDE/Codex/VS Code/Windsurf/Kiro/Cursor/Grok CLI/CodeBuddy/CodeBuddy CN/Qoder/Trae/Zed/OpenCode App Path | Manually set executable path | Leave empty (auto-detect) | Change only if auto-detect fails or you use custom install paths |
| Auto-restart OpenCode on Codex switch | Sync OpenCode auth after Codex switch | ON if you use OpenCode; otherwise OFF | Enable for frequent Codex switching with OpenCode |

Observações:
- Intervalos de atualização menores significam solicitações mais frequentes.
- Se as tarefas de ativação para redefinição de cota estiverem ativadas, alguns limites mínimos de atualização podem ser aplicados (a interface do usuário exibirá dicas).

### Configurações de Rede

| Configuração | Objetivo (simples) | Recomendado | Riscos/Observações |
| --- | --- | --- | --- |
| Serviço WebSocket | Integração local em tempo real para plugins/clientes | DESATIVADO se não for necessário | Ainda somente local (`127.0.0.1`) quando ativado |
| Porta Preferencial | Porta de escuta para WebSocket | Padrão `19528` | Alterar somente em caso de conflito; reinicialização necessária após salvar |
| Porta Atual | A porta ativa real | Informação somente leitura | Pode ser diferente se a porta preferencial estiver ocupada |

### 3 Predefinições Prontas para Uso

1. **Padrão estável**: atualização a cada 10 minutos, WebSocket DESATIVADO (se nenhum plugin estiver instalado), manter os caminhos padrão.
2. **Alternância frequente**: atualização a cada 2-5 minutos, WebSocket ATIVADO se necessário, sincronização com OpenCode ATIVADA.
3. **Prioridade à segurança**: WebSocket DESATIVADO, não compartilhar diretório de usuários, remover contas não utilizadas regularmente.

---



---

## Guia de Instalação

### Opção A: Download Manual (Recomendado)

Acesse a [página de lançamentos do GitHub](https://github.com/jlcodes99/cockpit-tools/releases) para baixar o pacote para o seu sistema:

*   **macOS**: `.dmg` (Apple Silicon & Intel)
*   **Windows**: `.msi` (Recommended) or `.exe`
*   **Linux**: `.deb` (Debian/Ubuntu), `.rpm`, or `.AppImage` (Universal)

### Opção B: Instalar com o Homebrew (macOS)

> É necessário o Homebrew.


```bash
brew tap jlcodes99/cockpit-tools https://github.com/jlcodes99/cockpit-tools
brew install --cask cockpit-tools
```

Se você se deparar com o aviso "O aplicativo está danificado" do macOS, também poderá instalar com a opção `--no-quarantine`:

```bash
brew install --cask --no-quarantine cockpit-tools
```

Se o Homebrew disser que o aplicativo já existe (por exemplo, `já existe um aplicativo em '/Applications/Cockpit Tools.app'`), remova o aplicativo antigo e instale-o novamente:

```bash
rm -rf "/Applications/Cockpit Tools.app"
brew install --cask cockpit-tools
```

Ou force a sobrescrita do aplicativo existente:

```bash
brew install --cask --force cockpit-tools
```

### 🛠️ Solução de problemas

#### O macOS exibe a mensagem "O app está danificado e não pode ser aberto"?
Devido aos mecanismos de segurança do macOS, apps que não foram baixados da App Store podem exibir este aviso. O fluxo de distribuição de código aberto atual ainda não utiliza a assinatura ou autenticação do ID de desenvolvedor da Apple, portanto, algumas versões do macOS podem exibir avisos mais rigorosos do Gatekeeper. Você pode corrigir isso rapidamente seguindo estas etapas:

1.  **Correção via linha de comando** (Recomendado):
    Abra o Terminal e execute o seguinte comando:
    ```bash
    sudo xattr -rd com.apple.quarantine "/Applications/Cockpit Tools.app"
    ```
    > **Observação**: Se você alterou o nome do aplicativo, ajuste o caminho no comando de acordo.

2.  **Ou**: Acesse "Configurações do Sistema" -> "Privacidade e Segurança" e clique em "Abrir mesmo assim".

---

## Desenvolvimento & Compilação

### Pré-requisitps

- Node.js v18+
- npm v9+
- Rust (Tauri runtime)

### Instalar Dependências

```bash
npm install
```

### Modo Desenvolvimento

```bash
npm run tauri dev
```

### Compilação

```bash
npm run tauri build
```

---

## Histórico do Star

[![Gráfico do Histórico do Star](https://api.star-history.com/svg?repos=jlcodes99/cockpit-tools&type=Date)](https://star-history.com/#jlcodes99/cockpit-tools&Date)

---

## Comunidade

Grupo de bate-papo no Telegram recém-criado: [Junte-se ao grupo](https://t.me/+Y8gMv4SlZUU2MWY1)

---

## Patrocínio

Se você achar este projeto útil, considere apoiá-lo aqui: [☕ Doar](docs/DONATE.pt-br.md)

Toda contribuição ajuda a manter o desenvolvimento de código aberto. Obrigado!

---

## Agradecimentos

- Lógica de troca de contas do Antigravity IDE baseada em: [Antigravity-Manager](https://github.com/lbjlaq/Antigravity-Manager)
- Implementação do serviço Codex API desenvolvida com referência a: [router-for-me/CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI)

Agradecemos ao autor do projeto por suas contribuições de código aberto! Se esses projetos lhe foram úteis, por favor, dê a eles uma ⭐ estrela para demonstrar seu apoio!

---

## Licença

Este projeto está licenciado sob a licença [CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/).

- Permitido: uso pessoal para fins de aprendizagem, pesquisa e uso/modificação não comercial (com atribuição e compartilhamento pela mesma licença).
- Não permitido: qualquer uso comercial sem autorização (incluindo operações comerciais internas, serviços pagos externos, integração com produtos pagos ou revenda/redistribuição com fins lucrativos).
- Licença comercial: entre em contato com o autor para obter uma licença comercial por escrito e os respectivos preços.

---

## Aviso Legal

Este projeto destina-se exclusivamente a fins de aprendizagem e pesquisa pessoal. Ao utilizar este projeto, você concorda em:

- Não utilizar este projeto para fins comerciais sem autorização prévia por escrito do autor.
- Assumir todos os riscos e responsabilidades decorrentes do uso deste projeto.
- Cumprir os termos de serviço, leis e regulamentos aplicáveis.

O autor do projeto não se responsabiliza por quaisquer perdas diretas ou indiretas resultantes do uso deste projeto.
