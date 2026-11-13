MASTER DIRECTIVE: EVOLVE THIS APPLICATION INTO A LOCAL KNOWLEDGE INTELLIGENCE ENGINE

YOUR ROLE

You are operating as a multidisciplinary principal engineering team consisting of:

Principal software architect.
Search engine architect.
Information retrieval specialist.
Machine learning engineer.
Local LLM integration specialist.
Knowledge graph architect.
Database and indexing specialist.
Performance optimization engineer.
Privacy and security engineer.
Human-computer interaction specialist.
Your mission is to deeply understand, challenge, evaluate, and ultimately help evolve this existing application into an exceptionally intelligent, high-performance, completely private local knowledge discovery system.

This application already exists and is substantially developed.

You are NOT being asked to build a new application.

You are being asked to identify how to make the existing application dramatically smarter, more capable, more context-aware, more accurate, and more efficient.

Think ambitiously. Think beyond traditional desktop search. Think beyond conventional semantic search.

The ultimate goal is to create something resembling a private, locally operating knowledge intelligence system for an entire computer.

A system that does not merely know where files are, but understands what they mean, how they relate to one another, what information they collectively contain, and how to retrieve and reason over that information.

The existing application is the foundation. Preserve its strengths and evolve it intelligently.

PART I — THE NON-NEGOTIABLE PHILOSOPHY

1. Absolute local-first intelligence

Everything must operate locally.

This is a fundamental architectural requirement.

The system must not depend on external AI services, cloud inference, external embedding APIs, or remote knowledge processing.

Requirements:

All embeddings generated locally.
All LLM inference performed locally.
All document processing performed locally.
All semantic indexing performed locally.
All knowledge graph operations performed locally.
All query understanding performed locally.
All reranking performed locally.
All document summaries generated locally.
All relationship inference performed locally.
All stored intelligence remains on the machine.
No telemetry. No analytics transmission. No silent cloud requests. No automatic uploads. No external fallback inference. No phoning home.

Model downloads may be supported through explicit user-initiated installation, but once installed, the application must operate without internet connectivity.

Audit every dependency and every component that could communicate externally.

The application must be demonstrably functional with network access disabled.

Treat privacy as an architectural property, not a settings toggle.

2. Intelligence must not compromise performance

The engine must feel responsive and lightweight.

I want sophisticated intelligence without turning the application into a resource-hungry AI application.

Prioritize:

Instant-feeling search.
Low-latency retrieval.
Incremental indexing.
Background intelligence generation.
Efficient embedding pipelines.
Intelligent caching.
Model reuse.
Minimal redundant processing.
Efficient CPU utilization.
Optional GPU acceleration.
Controlled memory usage.
Fast startup.
Graceful operation on CPU-only machines.
Never introduce an LLM into a workflow where a deterministic algorithm, database query, cache, or smaller model is sufficient.

Use intelligence selectively.

3. Preserve the existing application

Do not rewrite the application without strong evidence.

Understand its architecture before proposing changes.

Preserve:

Existing functionality.
Existing user workflows.
Existing interfaces.
Data compatibility.
Existing performance characteristics.
Existing indexing capabilities.
Existing integrations that are genuinely useful.
Any major architectural change must be justified by measurable benefits.

PART II — THE ULTIMATE VISION

4. From filesystem search to personal knowledge intelligence

I want the application to develop an understanding of the user's digital environment.

Consider a computer containing:

Documents.
PDFs.
Spreadsheets.
Presentations.
Images.
Screenshots.
Source code.
Configuration files.
Videos.
Audio recordings.
Archives.
Notes.
Projects.
Downloads.
Research materials.
Personal and professional files.
Traditional search sees these as individual filesystem objects.

I want this engine to understand them as interconnected pieces of information.

For example:

A folder called "Q4" might contain:

A budget spreadsheet.
A product proposal.
A presentation.
Meeting notes.
Several screenshots.
A PDF containing technical requirements.
The engine should be capable of inferring that these materials collectively describe a particular project or business initiative.

It should understand the relationships between these files even when their filenames are different and they are stored in different locations.

It should be able to answer questions about their combined contents.

The system should develop an evolving, searchable representation of the user's local knowledge.

This is the central vision.

PART III — COMPLETE REPOSITORY INVESTIGATION

5. Understand the existing application before changing anything

Perform an exhaustive investigation of the repository.

Inspect:

Application architecture.
All important source modules.
Search pipeline.
Indexing pipeline.
File discovery.
Filesystem watchers.
Database structure.
Full-text indexing.
Vector indexing.
Embedding generation.
Model inference.
Document parsing.
Metadata extraction.
Folder traversal.
Search ranking.
Query processing.
Caching.
Background workers.
Threading.
Concurrency.
UI communication.
Configuration.
Settings.
Logging.
Error handling.
Network dependencies.
Existing tests.
Existing performance benchmarks.
Trace the entire lifecycle of a file, from discovery to retrieval.

Verify that features described in documentation are actually implemented.

Identify:

What exists.
What works.
What is incomplete.
What is redundant.
What is inefficient.
What is architecturally limiting.
What could be significantly improved.
Produce a dependency and architecture map.

Do not modify anything during this investigation.

PART IV — THE INTELLIGENCE ARCHITECTURE

6. Build a multi-layer understanding of the filesystem

Investigate whether the existing architecture can evolve into the following conceptual layers.

Layer A: Filesystem intelligence

Understand the actual filesystem.

Each file should have a meaningful representation containing, where available:

Stable file identifier.
Filename.
Full path.
Extension.
File type.
Parent folder.
Folder hierarchy.
File size.
Creation time.
Modification time.
Content hash.
Extracted text.
Metadata.
Extracted entities.
Topics.
Semantic representation.
Related files.
Version relationships.
Avoid storing redundant information.

Layer B: Content intelligence

Understand the actual content of files.

Extract:

Main subjects.
Concepts.
Topics.
Named entities.
People and organizations where present.
Dates.
Important terminology.
References.
Document purpose.
Key information.
Project identifiers.
Relationships to other documents.
Investigate appropriate extraction methods for each supported file type.

Do not treat every file as plain text.

Layer C: Semantic intelligence

Create meaningful representations of files and their contents.

Investigate:

Local embedding models.
Document-aware embeddings.
Chunking strategies.
Context-enriched embeddings.
Query/document embedding compatibility.
Semantic similarity.
Hierarchical embeddings.
Folder embeddings.
Topic representations.
Multilingual retrieval if appropriate.
Determine the best balance between semantic quality, storage, model size, and inference speed.

Layer D: Relationship intelligence

Develop a system that understands how files relate.

Investigate the use of a local knowledge graph or an equivalent relational structure.

Potential entities:

Files.
Folders.
Projects.
Topics.
Concepts.
Organizations.
People.
Events.
Versions.
References.
Document clusters.
Potential relationships:

FILE_IN_FOLDER.
RELATED_TO.
SAME_PROJECT.
REFERENCES.
MENTIONS.
SAME_TOPIC.
VERSION_OF.
DUPLICATE_OF.
SUPPORTS.
CONTRADICTS.
CREATED_DURING.
DERIVED_FROM.
Relationships must have evidence and confidence scores.

Distinguish between:

Directly verified relationships.
Metadata-derived relationships.
Statistically inferred relationships.
LLM-inferred relationships.
Never present an uncertain relationship as an established fact.

Investigate efficient graph storage using the existing database before introducing another database system.

Layer E: Contextual intelligence

Understand the environment surrounding each file.

A file should not be interpreted in isolation.

Its context may include:

Its folder.
Its neighboring files.
Its parent project.
Similar documents.
Shared entities.
Its temporal context.
Its related documents.
Its position within a topic cluster.
Evaluate contextual representations that can be updated incrementally.

Layer F: Knowledge intelligence

Build the ability to retrieve and synthesize information across files.

The system should eventually answer questions such as:

What documents do I have about a particular project?
Which files relate to this spreadsheet?
What are the latest documents about a particular subject?
Which folders contain information about this topic?
What changed between two versions of a document?
Which documents support a particular decision?
What other documents might be relevant to this file?
What information is spread across several unrelated folders?
This layer must use retrieved source material and provide traceable references.

PART V — THE KNOWLEDGE GRAPH

7. Make relationships a first-class capability

I consider relationship intelligence one of the most important opportunities.

Investigate whether the engine should maintain an evolving local knowledge graph.

The graph should help answer questions that ordinary vector similarity cannot reliably solve.

For example:

Two files might have very different wording but refer to the same project.

Two spreadsheets might be related because they contain the same project identifier.

A PDF might describe requirements implemented in a source-code repository.

A meeting note might reference a decision documented in a separate presentation.

A group of files might collectively describe a project even though no folder explicitly identifies it.

Investigate:

Entity extraction.
Entity resolution.
Topic clustering.
Project discovery.
File relationship inference.
Folder relationships.
Document references.
Version detection.
Temporal relationships.
Duplicate detection.
Relationship confidence.
Evidence provenance.
Graph traversal.
Relationship updates.
Graph invalidation when files change.
Important:

Do not create relationships indiscriminately.

A large collection of weak or false relationships can make search worse.

Relationships should be weighted, explainable, and based on evidence.

Investigate how graph information could influence search ranking without overwhelming direct semantic relevance.

PART VI — LOCAL AI AND MACHINE LEARNING

8. Design an intelligent multi-model architecture

Do not build an LLM-centric system.

Use the smallest and fastest appropriate technology for each task.

Evaluate:

A. Embedding models

Investigate current practical local models from:

BGE families.
E5 families.
Qwen embedding families.
Other competitive local retrieval models.
Compare:

Retrieval quality.
Model size.
CPU performance.
GPU acceleration.
Memory requirements.
Embedding throughput.
Context length.
Multilingual support.
License.
Offline inference.
B. Reranking models

Evaluate local rerankers for improving the ordering of candidate results.

Determine whether a cross-encoder or another reranking strategy would provide measurable improvements.

Use reranking selectively on a small candidate set.

C. Local LLMs

Evaluate local LLMs for:

Query interpretation.
Entity extraction.
Document summaries.
Folder summaries.
Project identification.
Relationship inference.
Multi-document reasoning.
Search explanations.
Natural-language questions.
Consider quantized models and hardware-aware inference.

Evaluate runtimes such as:

llama.cpp.
ONNX Runtime.
Other appropriate local inference frameworks.
Do not select technologies simply because they are well known.

D. Lightweight intelligence

Identify tasks that can be accomplished using:

Rules.
Statistical methods.
Text classification.
Clustering.
Hashing.
Metadata analysis.
Traditional information retrieval.
Small embedding models.
The objective is maximum intelligence per unit of compute.

E. Adaptive inference

Design a query and processing router.

Examples:

Exact filename search: keyword index.
Natural-language concept search: vector index.
Complex search: query understanding plus hybrid retrieval.
Related-file discovery: relationship index.
Multi-document question: local LLM with retrieved evidence.
Repeated query: cached results where safe.
Model inference should be triggered only when justified.

PART VII — THE SEARCH EXPERIENCE

9. Create a genuinely intelligent search engine

The search system should support more than keyword matching.

Investigate the following capabilities.

Natural-language search

Examples:

"Find the document where I discussed changing the application architecture."

"Show me the spreadsheets related to the website redesign."

"Find screenshots from when I was working on the new dashboard."

"Where are the documents discussing the budget for that project?"

"Find the latest version of my technical specifications."

Conceptual search

Understand intent even when the query does not share exact words with the document.

Contextual search

Use folder structure, neighboring files, project context, and document relationships.

Relationship search

Discover files related to another file, topic, person, project, or entity.

Temporal search

Understand:

Latest.
Oldest.
Recently modified.
From a particular period.
Before or after another document.
Likely version sequences.
Search filters

Support natural-language interpretation of:

File type.
Folder.
Date.
Size.
Project.
Topic.
Entity.
Relationship.
Search explanations

For each result, explain why it appeared.

Possible reasons:

Exact filename match.
Content match.
Semantic similarity.
Same project.
Shared entity.
Related folder.
Document reference.
Temporal relationship.
The user should be able to understand why the engine considers something relevant.

PART VIII — MULTI-DOCUMENT REASONING

10. Move beyond finding files toward answering questions

Investigate a local retrieval-augmented generation architecture.

The engine should eventually answer questions using multiple indexed documents.

Example:

"Summarize everything I have about the website redesign project."

The engine should:

Identify the likely project.
Find associated folders and files.
Retrieve relevant documents.
Retrieve related documents outside the primary folder.
Identify overlapping information.
Detect conflicting information where possible.
Organize the evidence.
Generate a local answer.
Cite the exact source files.
Allow the user to open the original files.
Another example:

"What decisions were made about the database architecture, and which documents discuss them?"

The engine should find supporting documents, extract the relevant passages, and distinguish confirmed evidence from inference.

Important safeguards:

Never hallucinate file contents.
Never fabricate citations.
Distinguish retrieved evidence from generated interpretation.
Preserve source paths and document references.
Respect file access permissions.
Do not send content outside the machine.
Evaluate whether multi-document reasoning can be performed efficiently with a local model and bounded context windows.

PART IX — ADVANCED KNOWLEDGE DISCOVERY

11. Investigate the following advanced capabilities

These are potential differentiators, not mandatory feature additions.

Project intelligence

Automatically identify groups of files that appear to belong to the same project.

Topic discovery

Identify emerging topics and organize documents into meaningful clusters.

Related-file discovery

For any selected file, find other relevant files across the filesystem.

Semantic folder summaries

Create concise descriptions of what folders contain based on their actual contents.

Duplicate intelligence

Identify:

Exact duplicates.
Near-duplicate documents.
Modified versions.
Similar content in different formats.
Document evolution

Understand how documents change over time and identify likely revisions.

Entity intelligence

Discover recurring organizations, products, people, technologies, and other named entities.

Visual intelligence

Evaluate local OCR and image understanding for screenshots, scanned PDFs, diagrams, and images.

Do not assume every image needs a large vision model.

Code intelligence

If source code is present, investigate:

Symbol extraction.
Function and class relationships.
Import relationships.
Project structure.
Documentation links.
Semantic code search.
Temporal intelligence

Understand how files, projects, and topics evolve over time.

Knowledge recommendations

Suggest related files when opening or searching for a document.

Knowledge gaps

Investigate whether the engine can identify incomplete project documentation, missing references, or potentially disconnected information.

Only implement such capabilities when their usefulness can be demonstrated.

PART X — PERFORMANCE ENGINEERING

12. Make the engine exceptionally efficient

Conduct a complete performance audit.

Measure and optimize:

Startup.
Initial indexing.
Incremental indexing.
Search latency.
Embedding generation.
Model initialization.
Database operations.
Vector retrieval.
Graph traversal.
Memory utilization.
CPU utilization.
GPU utilization.
Thread contention.
Disk I/O.
Cache hit rates.
Background processing.
Large-directory behavior.
Evaluate scenarios involving:

10,000 files.
100,000 files.
500,000 files.
1 million files.
Do not assume all files need embeddings.

Investigate:

Selective embedding.
Hierarchical retrieval.
Coarse-to-fine search.
Lazy indexing.
Background enrichment.
Incremental graph construction.
Batch processing.
Content hashing.
Embedding reuse.
Query caching.
Folder-level aggregation.
The search experience should remain responsive while indexing continues in the background.

Create measurable performance targets based on the actual implementation and available hardware.

Do not invent benchmark results.

PART XI — DATABASE AND STORAGE ARCHITECTURE

13. Audit how information is stored

Investigate whether the current storage architecture can support:

File metadata.
Full-text indexes.
Vector embeddings.
Document chunks.
Folder representations.
Entities.
Topics.
Relationships.
Relationship evidence.
Summaries.
Model metadata.
Index versions.
Incremental updates.
Evaluate SQLite, FTS5, existing vector stores, and graph-like relational representations against the current stack.

Avoid introducing unnecessary database systems.

Investigate:

Index fragmentation.
Query plans.
Database locking.
Write amplification.
Storage overhead.
Migration safety.
Transaction boundaries.
Data integrity.
Backup and recovery.
Corruption recovery.
All derived intelligence must be reproducible from the original files and indexing configuration wherever practical.

PART XII — PRIVACY AND SECURITY

14. Prove that the application is private

Perform an actual network and dependency audit.

Inspect:

HTTP clients.
Model downloaders.
Telemetry.
Analytics.
Crash reporting.
Update checkers.
Package dependencies.
Background services.
External integrations.
Remote inference configurations.
Determine whether any of these can transmit user information.

Create a verification strategy that includes:

Running with network access disabled.
Monitoring outbound connection attempts.
Testing search functionality offline.
Testing local model inference offline.
Testing indexing offline.
Testing updates and failures without external access.
Verifying that no cloud fallback occurs.
The system must not expose sensitive filenames, file paths, document content, queries, or embeddings.

Also evaluate local data security and whether sensitive indexed content should be protected at rest.

PART XIII — INTELLIGENT SYSTEM DESIGN

15. Build a layered intelligence pipeline

Evaluate a target architecture along these lines:

FILE DISCOVERY | v METADATA EXTRACTION | v CONTENT EXTRACTION | v SEMANTIC REPRESENTATION | v ENTITY AND TOPIC EXTRACTION | v RELATIONSHIP DISCOVERY | v KNOWLEDGE INDEX | v HYBRID RETRIEVAL | v CONTEXTUAL RANKING | v OPTIONAL RERANKING | v OPTIONAL LOCAL LLM REASONING | v EXPLAINABLE RESULTS

This is a conceptual architecture, not a requirement to implement every layer as a separate service.

Investigate which components can be combined within the existing application.

Optimize for:

Simplicity.
Reliability.
Low latency.
Incremental processing.
Observability.
Maintainability.
Local execution.
PART XIV — EVALUATION AND BENCHMARKING

16. Define what intelligence actually means

Do not use vague claims such as "the search is much smarter."

Create measurable evaluation criteria.

Evaluate:

Retrieval quality

Precision@K.
Recall@K.
MRR.
nDCG.
Exact-match accuracy.
Semantic retrieval accuracy.
Cross-folder retrieval accuracy.
Relationship retrieval accuracy.
Contextual understanding

Project association accuracy.
Entity extraction accuracy.
Folder classification accuracy.
Relationship precision.
Relationship recall.
Version detection accuracy.
Reasoning quality

Source attribution accuracy.
Factual consistency.
Multi-document answer quality.
Citation correctness.
Hallucination rate.
Performance

Search latency.
Indexing throughput.
Embedding throughput.
Memory consumption.
CPU utilization.
GPU utilization.
Startup time.
Privacy

Network activity.
External dependency audit.
Offline feature completeness.
Build repeatable test datasets from representative local files, with appropriate privacy protections.

Establish a baseline before implementing improvements.

Every meaningful change should be evaluated against that baseline.

PART XV — IMPLEMENTATION STRATEGY

17. Do not implement everything at once

After completing the audit, create an implementation roadmap.

Prioritize:

P0 — Critical correctness, privacy, and architectural issues.

P1 — High-impact improvements to search quality and contextual intelligence.

P2 — Performance, indexing, storage, and scalability.

P3 — Advanced knowledge graph and AI capabilities.

P4 — Advanced discovery, reasoning, and user experience.

For every proposed improvement, document:

Existing implementation.
Identified limitation.
Recommended change.
Expected benefit.
Complexity.
Dependencies.
Performance impact.
Storage impact.
Privacy implications.
Test strategy.
Rollback strategy.
Implement in small, independently verifiable phases.

Do not introduce unnecessary rewrites.

Preserve compatibility with existing indexed data wherever practical.

PART XVI — AUTONOMOUS ENGINEERING EXPECTATIONS

18. Challenge assumptions and think independently

Do not blindly follow the existing architecture.

Do not blindly follow this proposed architecture either.

You are expected to challenge both.

If a simpler implementation is better, explain why.

If an existing component is already excellent, leave it alone.

If a proposed capability is too expensive or unlikely to improve the user experience, say so.

If a model is too large, find a smaller alternative.

If a knowledge graph would add complexity without meaningful retrieval gains, demonstrate that.

If the application has a hidden architectural weakness, identify it even if it is outside the areas explicitly listed here.

Do not confuse feature count with intelligence.

Do not confuse model size with intelligence.

Do not confuse architectural complexity with engineering quality.

Optimize for the actual outcome.

PART XVII — EXECUTION RULES

19. Initial investigation mode

For the first stage:

Read the repository.
Analyze the code.
Trace actual implementations.
Inspect configuration.
Inspect dependencies.
Examine database schemas.
Inspect tests.
Identify architectural opportunities.
Do not modify source files.

Do not install packages.

Do not download models.

Do not alter databases.

Do not execute destructive commands.

Do not make unapproved architectural changes.

Present the complete audit and roadmap first.

20. Implementation mode

Only after the audit and approval:

Work in controlled phases.
Establish performance baselines.
Make small changes.
Run tests.
Run relevance benchmarks.
Validate offline behavior.
Check regressions.
Document changes.
Preserve existing behavior.
Stop and report if a proposed change creates unexpected architectural risk.
Never silently remove existing capabilities.

PART XVIII — THE ULTIMATE SUCCESS CRITERIA

21. What I ultimately want this application to become

Imagine a user searching:

"Find everything I have related to the new application architecture."

The system should be able to:

Understand the meaning of the request.
Identify the relevant project.
Search across unrelated folder structures.
Find architecture documents.
Discover related source code.
Find relevant diagrams.
Identify meeting notes.
Find associated technical decisions.
Detect newer versions.
Identify related spreadsheets or planning documents.
Group the results intelligently.
Explain why each result is relevant.
Show the connections between them.
Now imagine the user selecting one document and asking:

"What else on my computer is connected to this?"

The engine should discover related information using content, entities, project context, references, semantic similarity, and its knowledge graph.

Now imagine asking:

"Summarize everything I know about this project."

The system should retrieve the relevant knowledge, synthesize it locally, cite its sources, and distinguish facts from inferred relationships.

This is the experience I want to build.

The engine should progressively develop a useful understanding of the user's digital knowledge without ever requiring that knowledge to leave the machine.

It should be:

Intelligent.
Context-aware.
Relationship-aware.
Semantically capable.
Explainable.
Fast.
Scalable.
Private.
Reliable.
Resource-efficient.
Continuously updatable.
FINAL DIRECTIVE

Do not approach this as a routine code review.

Approach it as the evolution of an existing search product into a serious local knowledge intelligence platform.

Think deeply about architecture, information retrieval, machine learning, knowledge representation, local inference, performance engineering, and user experience.

Challenge the current implementation.

Find opportunities that may not be obvious.

Identify the highest-impact improvements.

Be technically rigorous.

Be ambitious about intelligence and conservative about unnecessary complexity.

The objective is not to bolt an AI chatbot onto a file search application.

The objective is to build an engine that understands a user's local digital world, discovers relationships between its contents, and makes that knowledge instantly searchable and usable — entirely on the user's own machine.

Begin by investigating the entire repository.

Do not change anything yet.

Return a comprehensive technical assessment, architecture map, intelligence gap analysis, privacy audit, benchmark plan, and prioritized implementation roadmap.