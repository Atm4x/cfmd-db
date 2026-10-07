# CFMD — REPORT 2
# Schema-Owned Access DX: Access как часть authoritative Schema

## Статус

Это выбранный целевой DX, а не набор альтернатив.

Основная идея:

> **Authorization не является отдельной системой рядом со Schema. `Access` является частью authoritative Schema contract.**
>
> Все права описываются только через typed DbSet/EntitySet-координаты этой конкретной Schema.  
> Migration работает над теми же schema coordinates и поэтому обязана одновременно проверить перенос data semantics и access semantics.

Снаружи разработчик видит:

```text
Schema
├── DbSets / EntitySets
├── Rules / invariants
└── Access
    ├── Capabilities
    └── Roles
```

Внутри runtime всё по-прежнему компилируется в существующий:

```text
RoleId[]
    -> AccessCapabilityId[]
    -> exact PermissionCoordinate[]
    -> PermissionSet
    -> existing P503-P515 enforcement
```

Нового authorization evaluator не появляется.

---

# 1. Главный закон

`Access` нельзя создать, открыть, смигрировать или интерпретировать отдельно от конкретной authoritative Schema.

Нельзя:

```rust
let policy = AuthorizationPolicy::new(...);
db.attach_policy(policy);
```

и нельзя:

```python
auth = authorization(...)
db.use(auth)
```

как основной публичный mental model.

Вместо этого:

```text
SchemaA.Access
```

является такой же частью определения SchemaA, как:

```text
SchemaA.users
SchemaA.orders
SchemaA.rules
```

Следствие:

```text
SchemaA.Access
    может ссылаться только на SchemaA DbSets

SchemaB.Access
    может ссылаться только на SchemaB DbSets
```

Stale reference из A в B должен быть compile-time / definition-time error.

---

# 2. Почему DbSet должен стать canonical authorization coordinate frontend

У CFMD уже есть object-first schema root:

```rust
#[derive(CfmdSchema)]
struct AppSchema {
    users: EntitySet<User>,
    tickets: EntitySet<Ticket>,
}
```

Для разработчика именно:

```text
schema.users
schema.tickets
```

являются естественным способом сказать:

> «вот Users именно этой Schema».

Поэтому Access не должен писать:

```text
raw RelationId
raw FieldId
"users.email"
User::email из глобального type namespace
```

как основной DX.

Он должен работать через schema-bound coordinate surface:

```text
schema.users.email
schema.tickets.status
schema.tickets.comments
```

Compiler уже затем переводит это в:

```text
RelationId / FieldCoordinate / RelationshipCoordinate / GlobalCoordinate
```

То есть один и тот же semantic-addressing слой используется для:

```text
Schema definition
Access
Migration
Query binding
History/Watch coordinate binding
```

Это резко уменьшает количество параллельных naming systems.

---

# 3. Финальный Rust mental model

Целевой high-level frontend:

```rust
cfmd::schema! {
    pub struct AppSchema {
        users: EntitySet<User>,
        tickets: EntitySet<Ticket>,

        access {
            capability CustomerContactRead("access.customer.contact.read") {
                read users::{name, email, phone};
            }

            capability CustomerContactEdit("access.customer.contact.edit") {
                write users::{email, phone};
            }

            capability TicketRead("access.ticket.read") {
                read tickets::{id, subject, status};
            }

            capability TicketStatusSet("access.ticket.status.set") {
                write tickets::status;
            }

            capability TicketCommentsAttach("access.ticket.comments.attach") {
                attach tickets::comments;
            }

            role SupportL1("support.l1") {
                CustomerContactRead;
                TicketRead;
                TicketStatusSet;
                TicketCommentsAttach;
            }

            role SupportL2("support.l2") {
                CustomerContactRead;
                CustomerContactEdit;
                TicketRead;
                TicketStatusSet;
                TicketCommentsAttach;
            }
        }
    }
}
```

Здесь важно не конкретное имя macro `schema!`, а закон DX:

```text
access block lexical-owner = AppSchema
```

и внутри:

```text
users::email
```

может означать только:

```text
AppSchema.users.email
```

Никакого глобального поиска по имени.

---

# 4. Что macro реально генерирует

High-level syntax не должен создавать новый semantic engine.

Compiler lowering:

```text
AppSchema
  access {
      capability ...
      role ...
  }

        ↓

existing kernel-schema AuthorizationPolicy

        ↓

AccessCapabilityDef {
    stable_id,
    exact PermissionCoordinate[]
}

AuthorizationRoleDef {
    stable_id,
    capability_ids[],
    included_role_ids[]
}

        ↓

SchemaView::permissions_for_roles(...)

        ↓

PermissionSet
```

То есть нынешний runtime authorization substrate сохраняется.

Меняется ownership и authoring surface.

---

# 5. Capability остаётся обязательной semantic boundary между Role и fields

Role не должна писать:

```rust
role Support {
    read users::email;
    read users::phone;
    write tickets::status;
}
```

Основной DX:

```rust
role Support {
    CustomerContactRead;
    TicketStatusSet;
}
```

Почему:

```text
Role
    = business assignment vocabulary

Capability
    = stable security contract

Schema coordinate
    = current realization of that contract
```

Это позволяет тысячам roles переживать schema migration без массовой переписи.

---

# 6. AccessCapability получает более строгий смысл

`AccessCapabilityId` — stable lineage identity.

Например:

```text
access.customer.contact.read
```

Но stable ID не означает «можно молча менять его содержимое».

Нужно разделить:

```text
CapabilityId
    stable lineage identity

CapabilityContract
    security meaning

CapabilityRealization<S>
    exact PermissionCoordinate set in Schema S
```

Пример:

```text
CapabilityId:
    access.customer.contact.read

Contract:
    "read customer contact surface"

Realization in A:
    users.name
    users.email
    users.phone

Realization in B:
    profiles.name
    contacts.primary_email
    contacts.phone
```

Migration может менять realization.

Она не имеет права молча менять Contract.

---

# 7. Role также имеет stable identity, но mutable policy definition

Аналогично:

```text
RoleId:
    support.l1

RoleDefinition:
    {
        CustomerContactRead,
        TicketRead,
        TicketStatusSet
    }
```

External identity provider хранит только:

```text
principal -> RoleId[]
```

Например:

```text
alice -> ["support.l1"]
```

Если RoleDefinition расширилась, это mass policy change для всех principals с этим RoleId.

Поэтому schema/migration diff обязан показывать изменение role composition отдельно от schema transport.

---

# 8. Тысячи ролей

Для 1000+ roles основной source не должен требовать builder ceremony.

Целевой вид:

```rust
cfmd::schema! {
    pub struct EnterpriseSchema {
        users: EntitySet<User>,
        tickets: EntitySet<Ticket>,
        invoices: EntitySet<Invoice>,
        payments: EntitySet<Payment>,

        access {
            capabilities {
                CustomerContactRead("access.customer.contact.read") {
                    read users::{name, email, phone};
                }

                TicketRead("access.ticket.read") {
                    read tickets::{id, subject, status};
                }

                TicketStatusSet("access.ticket.status.set") {
                    write tickets::status;
                }

                InvoiceRead("access.invoice.read") {
                    read invoices::{id, number, amount, state};
                }

                PaymentRead("access.payment.read") {
                    read payments::{id, amount, state};
                }
            }

            roles {
                SupportL1("support.l1") {
                    CustomerContactRead;
                    TicketRead;
                    TicketStatusSet;
                }

                SupportL2("support.l2") {
                    CustomerContactRead;
                    CustomerContactEdit;
                    TicketRead;
                    TicketStatusSet;
                }

                FinanceAudit("finance.audit") {
                    InvoiceRead;
                    PaymentRead;
                    AuditHistoryRead;
                }

                // ... ещё 997 roles
            }
        }
    }
}
```

Информационный объём 1000 независимых roles убрать невозможно.

Но ceremony сокращается до нижней границы:

```text
RoleId + capability set
```

---

# 9. Большой role catalog можно физически вынести из файла, не превращая его в отдельную систему

Одна Schema может быть слишком большой.

Поэтому разрешён source partitioning:

```rust
cfmd::schema! {
    pub struct EnterpriseSchema {
        users: EntitySet<User>,
        tickets: EntitySet<Ticket>,
        invoices: EntitySet<Invoice>,

        access {
            capabilities {
                // schema-sensitive definitions here
            }

            roles include!("access/roles.cfmd");
        }
    }
}
```

Но `roles.cfmd` не является самостоятельным policy document.

Он компилируется **только внутри** `EnterpriseSchema::Access` и может ссылаться только на capability symbols этого Access scope.

То есть:

```text
physical file separation != authority separation
```

Нельзя:

```text
load roles.cfmd
attach to another DB
```

без повторной компиляции против конкретной Schema.

---

# 10. Migration использует тот же DbSet coordinate language

Исходная Schema:

```rust
cfmd::schema! {
    pub struct SchemaA {
        users: EntitySet<UserA>,

        access {
            capability CustomerContactRead("access.customer.contact.read") {
                read users::{name, email, phone};
            }

            role Support("support") {
                CustomerContactRead;
            }
        }
    }
}
```

Новая:

```rust
cfmd::schema! {
    pub struct SchemaB {
        profiles: EntitySet<Profile>,
        contacts: EntitySet<Contact>,

        access {
            capability CustomerContactRead("access.customer.contact.read") {
                read profiles::name;
                read contacts::{primary_email, phone};
            }

            role Support("support") {
                CustomerContactRead;
            }
        }
    }
}
```

Migration:

```rust
migration!(SchemaA => SchemaB, |from, to, m| {
    m.move_field(
        from.users.name,
        to.profiles.name,
    );

    m.move_field(
        from.users.email,
        to.contacts.primary_email,
    );

    m.move_field(
        from.users.phone,
        to.contacts.phone,
    );
});
```

Здесь migration не знает строк:

```text
"User.email"
"Contact.primary_email"
```

Она работает теми же typed Schema coordinates:

```text
from.users.email
to.contacts.primary_email
```

---

# 11. Ключевой migration theorem

Compiler знает:

```text
Access_A.CustomerContactRead
    =
    READ {
        A.users.name,
        A.users.email,
        A.users.phone
    }
```

Migration M даёт exact coordinate transport:

```text
A.users.name  -> B.profiles.name
A.users.email -> B.contacts.primary_email
A.users.phone -> B.contacts.phone
```

Значит:

```text
Transport_M(Access_A.CustomerContactRead)
    =
    READ {
        B.profiles.name,
        B.contacts.primary_email,
        B.contacts.phone
    }
```

Compiler сравнивает с authoritative target declaration:

```text
Access_B.CustomerContactRead
```

Если exact security contract совпадает:

```text
AUTO PRESERVED
```

Никакого дополнительного authorization migration code нет.

---

# 12. Почему target Schema всё равно должна содержать Access declaration

Можно спросить:

> если migration сама умеет перенести Access, зачем писать Access в SchemaB?

Потому что SchemaB должна быть полностью authoritative сама по себе.

Нужно поддерживать:

```rust
Database::builder(path)
    .create_authoritative::<SchemaB>()?;
```

на новой пустой БД, где никакой SchemaA и migration history никогда не существовали.

Следовательно:

```text
SchemaB
```

обязана самостоятельно определять:

```text
- data shape
- rules
- access contracts
```

Migration не создаёт B.Access как единственный источник истины.

Она **проверяет**, что A.Access корректно переходит в самостоятельно определённый B.Access.

Это важное различие:

```text
B.Access = authority

Migration = proof that A.Access -> B.Access is valid
```

---

# 13. Rename / move — automatic

Пример:

```text
A.users.email
    ->
B.contacts.primary_email
```

если migration certified как semantic identity/move.

Capability:

```text
access.customer.contact.read
```

может остаться тем же.

Migration output:

```text
ACCESS TRANSPORT

CustomerContactRead
    PRESERVED

A:
    READ users.email

B:
    READ contacts.primary_email

reason:
    exact schema-coordinate identity transport
```

Role catalog: 0 изменений.

---

# 14. Same name / different meaning — не automatic

A:

```text
users.status = employment status
```

B:

```text
users.status = online-presence status
```

Даже если spelling одинаков:

```text
users.status
```

без migration semantic identity transport старый capability не переносится.

```text
CustomerEmploymentStatusRead
    -> NOT REPRESENTABLE
```

Новое поле должно использовать новую access semantics:

```text
PresenceStatusRead
```

или explicit policy evolution.

Name matching никогда не является authority.

---

# 15. New field не получает grants автоматически

SchemaB:

```rust
struct UserB {
    name: String,
    email: String,
    private_notes: String, // NEW
}
```

Если B.Access:

```rust
capability CustomerRead("access.customer.read") {
    read users::{name, email};
}
```

то:

```text
private_notes
```

не входит ни в какой старый permission.

Даже если была capability:

```text
CustomerRead
```

она не означает:

```text
read users::*
```

Wildcard over future schema surface запрещён как default law.

---

# 16. Policy widening отличается от migration

SchemaA:

```rust
capability CustomerContactRead("access.customer.contact.read") {
    read users::{email, phone};
}
```

SchemaB:

```rust
capability CustomerContactRead("access.customer.contact.read") {
    read contacts::{primary_email, phone, home_address};
}
```

Migration транспортирует только:

```text
email -> primary_email
phone -> phone
```

Следовательно:

```text
transported A:
    primary_email
    phone

declared B:
    primary_email
    phone
    home_address
```

Compiler классифицирует:

```text
POLICY WIDENING
```

а не:

```text
SCHEMA MIGRATION
```

Migration validation должна остановиться.

---

# 17. Explicit approval для реального policy change

Если widening намеренный:

```rust
migration!(SchemaA => SchemaB, |from, to, m| {
    m.move_field(
        from.users.email,
        to.contacts.primary_email,
    );

    m.move_field(
        from.users.phone,
        to.contacts.phone,
    );

    m.approve_access_change(
        from.access.CustomerContactRead,
        to.access.CustomerContactRead,
    );
});
```

Это не переносит permissions вручную.

Это лишь explicit security acknowledgement:

> «Я видел exact diff между source contract и target contract и подтверждаю его».

Migration preview:

```text
APPROVED ACCESS CHANGE

Capability:
    access.customer.contact.read

transported source:
    READ contacts.primary_email
    READ contacts.phone

target:
    READ contacts.primary_email
    READ contacts.phone
    READ contacts.home_address

delta:
    + READ contacts.home_address

affected roles:
    support.l1
    support.l2
    crm.viewer
    crm.editor
    ... 423 more
```

Для CI можно требовать отдельный security approval artifact, но это уже product workflow, не новый semantic engine.

---

# 18. Split / merge — fail closed by default

A:

```text
users.full_name
```

B:

```text
profiles.first_name
profiles.last_name
```

Data migration:

```rust
m.transform(
    from.users.full_name,
    (to.profiles.first_name, to.profiles.last_name),
    split_name,
);
```

не означает автоматически:

```text
READ full_name
==
READ first_name + READ last_name
```

Потому что B теперь может дать доступ к компонентам независимо.

Поэтому:

```text
authority-shape changed
```

и migration validation требует explicit approval.

Для DB 1.0 automatic authorization transport лучше ограничить:

```text
1:1 semantic identity / rename / move / relation retarget
```

Всё, что меняет cardinality или decomposition access footprint:

```text
split
merge
derived field
one -> many
many -> one
```

должно быть fail-closed до explicit decision.

---

# 19. Relationship access использует Schema DbSet path

Например:

```rust
cfmd::schema! {
    struct AppSchema {
        users: EntitySet<User>,
        teams: EntitySet<Team>,

        access {
            capability TeamMembershipEdit("access.team.membership.edit") {
                attach teams::members;
                detach teams::members;
            }
        }
    }
}
```

Developer не знает internal relation ID.

Compiler знает:

```text
teams::members
    -> generated ManyField<Team, User>
    -> exact relationship semantic coordinate
    -> attach/detach PermissionCoordinate
```

При migration relation move/retarget проходит через тот же verified migration mapping.

---

# 20. Global operations тоже принадлежат Schema.Access

Пример:

```rust
access {
    capability LiveObserve("access.live.observe") {
        watch;
    }

    capability AuditHistory("access.audit.history") {
        history_read;
    }

    capability SchemaOperator("access.schema.migrate") {
        schema_migrate;
    }
}
```

Они не DbSet-scoped, но всё равно являются members конкретной authoritative Schema.Access.

---

# 21. Rust low-level escape hatch

High-level DX не должен блокировать advanced use.

Можно сохранить generated builder:

```rust
impl SchemaAccess for AppSchema {
    fn build_access(s: SchemaAccessView<Self>) -> AccessDefinition<Self> {
        let contact_read = s
            .capability("access.customer.contact.read")
            .read(s.users.name)
            .read(s.users.email)
            .read(s.users.phone);

        s.role("support.l1")
            .grant(contact_read)
            .finish()
    }
}
```

Но это не основной путь для 1000 roles.

Главный public authoring surface остаётся declarative schema-owned block.

---

# 22. Python — тот же mental model

Python не должен иметь отдельный:

```python
auth = authorization(...)
```

который потом attach'ится к DB.

Целевой API:

```python
class AppSchema(Schema):
    users = DbSet(User)
    tickets = DbSet(Ticket)

    @access
    def Access(s):
        CustomerContactRead = s.capability(
            "access.customer.contact.read"
        ).read(
            s.users.name,
            s.users.email,
            s.users.phone,
        )

        CustomerContactEdit = s.capability(
            "access.customer.contact.edit"
        ).write(
            s.users.email,
            s.users.phone,
        )

        TicketRead = s.capability(
            "access.ticket.read"
        ).read(
            s.tickets.id,
            s.tickets.subject,
            s.tickets.status,
        )

        TicketStatusSet = s.capability(
            "access.ticket.status.set"
        ).write(
            s.tickets.status,
        )

        s.role(
            "support.l1",
            CustomerContactRead,
            TicketRead,
            TicketStatusSet,
        )

        s.role(
            "support.l2",
            CustomerContactRead,
            CustomerContactEdit,
            TicketRead,
            TicketStatusSet,
        )
```

Ключевой объект:

```python
s
```

— не Database и не runtime Context.

Это compile/definition-time `SchemaAccessView[AppSchema]`.

Он содержит только DbSets этой Schema:

```text
s.users
s.tickets
```

и generated fields/relationships внутри них.

Нельзя:

```python
s.other_schema.users
```

Нельзя передать field descriptor из другой Schema revision.

---

# 23. Python migration

SchemaA:

```python
class SchemaA(Schema):
    users = DbSet(UserA)

    @access
    def Access(s):
        CustomerContactRead = s.capability(
            "access.customer.contact.read"
        ).read(
            s.users.name,
            s.users.email,
            s.users.phone,
        )

        s.role("support", CustomerContactRead)
```

SchemaB:

```python
class SchemaB(Schema):
    profiles = DbSet(Profile)
    contacts = DbSet(Contact)

    @access
    def Access(s):
        CustomerContactRead = s.capability(
            "access.customer.contact.read"
        ).read(
            s.profiles.name,
            s.contacts.primary_email,
            s.contacts.phone,
        )

        s.role("support", CustomerContactRead)
```

Migration:

```python
with migration(SchemaA, SchemaB) as m:
    m.move(
        m.source.users.name,
        m.target.profiles.name,
    )

    m.move(
        m.source.users.email,
        m.target.contacts.primary_email,
    )

    m.move(
        m.source.users.phone,
        m.target.contacts.phone,
    )
```

После validate:

```text
CustomerContactRead: PRESERVED
support:             UNCHANGED
```

---

# 24. Почему Python Access — method, а не nested class

Такой код:

```python
class AppSchema(Schema):
    users = DbSet(User)

    class Access:
        ...
```

визуально красивый, но Python nested class body не является нормальным lexical closure над outer class namespace.

Это толкает API обратно к:

```text
strings
global descriptors
late setattr tricks
metaclass magic
```

Поэтому:

```python
@access
def Access(s):
```

лучше:

- `s` явно schema-bound;
- IDE видит typed/generated `s.users`;
- definition выполняется один раз при schema compilation;
- нельзя случайно взять coordinate из другой Schema;
- semantics одинаковы с Rust schema access scope.

---

# 25. Large Python role catalog

Capabilities остаются schema-sensitive:

```python
@access
def Access(s):
    CustomerContactRead = ...
    TicketRead = ...
```

Для тысяч roles можно:

```python
@access
def Access(s):
    CustomerContactRead = ...
    TicketRead = ...

    s.roles_from("access/roles.pycfmd")
```

Фрагмент:

```python
role("support.l1",
    CustomerContactRead,
    TicketRead,
    TicketStatusSet,
)

role("support.l2",
    CustomerContactRead,
    CustomerContactEdit,
    TicketRead,
    TicketStatusSet,
)

role("finance.audit",
    InvoiceRead,
    PaymentRead,
    AuditHistoryRead,
)
```

Но этот fragment компилируется только в scope конкретного `AppSchema.Access`.

Он не является independently attachable authorization system.

---

# 26. Creation law

Authoritative creation:

```rust
Database::builder("app.cfmd")
    .create_authoritative::<AppSchema>()?;
```

компилирует сразу:

```text
DbSets
Rules
Access
```

в один authoritative schema definition.

Нельзя создать:

```text
AppSchema data without AppSchema.Access
```

если Access объявлен required member этой schema contract.

Checkpoint/reopen сохраняет ту же compiled access semantics.

---

# 27. Open / Context law

После открытия:

```rust
let db = Database::open("app.cfmd")?;
```

Database остаётся schema-neutral runtime handle.

Typed admission:

```rust
let ctx = db.context::<AppSchema>()?;
```

проверяет schema contract, но authorization по-прежнему определяется Session.

То есть нельзя спутать:

```text
Context<AppSchema>
    = typed language / shape

Session PermissionSet
    = what principal may actually do
```

Schema owns access definitions.

Session owns current principal grant realization.

---

# 28. External identity provider

Не меняется:

```text
principal -> RoleId[]
```

например:

```rust
let session = db.session_for_roles(
    principal,
    [
        RoleId::of("support.l1"),
        RoleId::of("audit.viewer"),
    ],
)?;
```

Current authoritative Schema.Access резолвит эти RoleId.

Если role отсутствует:

```text
fail closed
```

Principal-role membership не мигрируется вместе со Schema.

---

# 29. Что происходит при Schema migration с active sessions

После cutover A -> B:

```text
same external RoleId[]
    ↓
B.Access RoleDefinitions
    ↓
B capability realizations
    ↓
new PermissionSet generation
```

То есть:

```text
"support.l1"
```

остаётся assignment identity.

Но runtime permissions refresh/revalidation должны использовать только current authoritative B.Access.

Никакой A.Access current-world authority не остаётся.

---

# 30. Migration preview должен иметь Access как first-class section

Пример:

```text
CFMD MIGRATION A -> B

DATA
  17 fields identity-preserved
   3 fields moved
   1 relation retargeted

RULES
   8 preserved
   0 changed

ACCESS
  capabilities:
    34 preserved exactly
     2 retargeted by schema identity
     1 policy widening
     0 unrepresentable

  roles:
    412 unchanged
      1 definition widened
      0 removed

BLOCKED:
  CustomerContactRead
      + READ contacts.home_address
      affects 427 roles

  SupportL1
      + PasswordReset
      affects all principals assigned RoleId "support.l1"
```

Migration нельзя publish до:

```text
unresolved access changes == 0
```

---

# 31. Compile-time / definition-time errors

## Cross-schema coordinate

```rust
SchemaB.access {
    read SchemaA::users::email;
}
```

Ошибка:

```text
AccessCoordinateOutsideSchema {
    owner: SchemaB,
    coordinate_owner: SchemaA,
}
```

## Unknown role capability

```rust
role Support {
    MissingCapability;
}
```

Ошибка compile/definition time.

## Capability ID reused with unrelated semantics

Fail migration validation.

## New field accidentally captured

Невозможно без explicit grant, потому что wildcard future-field grants отсутствуют.

## Removed coordinate

Capability становится unrepresentable, migration blocked.

---

# 32. Internal contract fingerprint

Публично developer не обязан видеть version numbers.

Внутренне compiler может хранить:

```text
CapabilityId
CapabilityContractFingerprint
CapabilityRealizationFingerprint
RoleDefinitionFingerprint
```

Это позволяет различить:

```text
same ID + same contract + new realization
```

от:

```text
same ID + changed contract
```

и:

```text
same RoleId + changed role composition
```

Это не public semantic versioning.

Это safety metadata для exact diff/approval/reopen.

---

# 33. Performance law

Всё schema/access resolution происходит:

```text
schema compile
migration validation
session construction / refresh
```

Не на каждом query row.

Hot path остаётся:

```text
operation exact footprint
    -> PermissionSet membership/set checks
```

Нельзя добавлять:

```text
per-row role resolution
per-query capability graph traversal
string path resolution
migration routing
old-schema access lookup
```

---

# 34. Почему это лучше отдельного AuthorizationPolicy

Отдельный `AuthorizationPolicy` создаёт неправильный mental model:

```text
Schema
+
Policy
```

и естественно провоцирует вопросы:

```text
какая policy сейчас attached?
можно ли применить старую policy к новой schema?
как policy знает rename?
что происходит при migration?
```

Schema-owned Access превращает ответ в тривиальный:

```text
SchemaB имеет только SchemaB.Access.
```

Migration уже обязана знать:

```text
A coordinate -> B coordinate
```

поэтому она естественно является единственным механизмом transport proof.

---

# 35. Почему Access не надо размещать прямо на каждом field

Не:

```rust
#[role(Support, read)]
email: String
```

Потому что:

- 1000 roles превратят Entity в security matrix;
- cross-entity capabilities размазываются;
- role composition становится неудобной;
- policy review становится field-centric вместо business-capability-centric;
- structural migration смешивает data shape и role assignments.

Правильная locality:

```text
Schema owns Access

Access refers to Schema DbSets
```

а не:

```text
Field owns Roles
```

---

# 36. Финальный Rust DX полностью

```rust
#[derive(CfmdEntity)]
#[cfmd(key = "app.user", authoritative)]
struct User {
    id: Id<User>,
    name: String,
    email: String,
    phone: String,
}

#[derive(CfmdEntity)]
#[cfmd(key = "app.ticket", authoritative)]
struct Ticket {
    id: Id<Ticket>,
    subject: String,
    status: TicketStatus,
    comments: Many<Comment>,
}

cfmd::schema! {
    pub struct AppSchema {
        users: EntitySet<User>,
        tickets: EntitySet<Ticket>,

        access {
            capabilities {
                CustomerContactRead("access.customer.contact.read") {
                    read users::{name, email, phone};
                }

                CustomerContactEdit("access.customer.contact.edit") {
                    write users::{email, phone};
                }

                TicketRead("access.ticket.read") {
                    read tickets::{id, subject, status};
                }

                TicketStatusSet("access.ticket.status.set") {
                    write tickets::status;
                }

                TicketCommentsEdit("access.ticket.comments.edit") {
                    attach tickets::comments;
                    detach tickets::comments;
                }

                LiveObserve("access.live.observe") {
                    watch;
                }
            }

            roles {
                SupportL1("support.l1") {
                    CustomerContactRead;
                    TicketRead;
                    TicketStatusSet;
                    TicketCommentsEdit;
                    LiveObserve;
                }

                SupportL2("support.l2") {
                    CustomerContactRead;
                    CustomerContactEdit;
                    TicketRead;
                    TicketStatusSet;
                    TicketCommentsEdit;
                    LiveObserve;
                }
            }
        }
    }
}
```

Создание:

```rust
let db = Database::builder("app.cfmd")
    .create_authoritative::<AppSchema>()?;
```

Session:

```rust
let session = db.session_for_roles(
    principal,
    [RoleId::of("support.l1")],
)?;
```

Context:

```rust
let mut ctx = db.context_with_session::<AppSchema>(&session)?;
```

---

# 37. Финальный migration DX полностью

SchemaB:

```rust
cfmd::schema! {
    pub struct AppSchemaV2 {
        profiles: EntitySet<Profile>,
        contacts: EntitySet<Contact>,
        tickets: EntitySet<TicketV2>,

        access {
            capabilities {
                CustomerContactRead("access.customer.contact.read") {
                    read profiles::name;
                    read contacts::{primary_email, phone};
                }

                CustomerContactEdit("access.customer.contact.edit") {
                    write contacts::{primary_email, phone};
                }

                TicketRead("access.ticket.read") {
                    read tickets::{id, subject, status};
                }

                TicketStatusSet("access.ticket.status.set") {
                    write tickets::status;
                }

                TicketCommentsEdit("access.ticket.comments.edit") {
                    attach tickets::comments;
                    detach tickets::comments;
                }

                LiveObserve("access.live.observe") {
                    watch;
                }
            }

            roles {
                SupportL1("support.l1") {
                    CustomerContactRead;
                    TicketRead;
                    TicketStatusSet;
                    TicketCommentsEdit;
                    LiveObserve;
                }

                SupportL2("support.l2") {
                    CustomerContactRead;
                    CustomerContactEdit;
                    TicketRead;
                    TicketStatusSet;
                    TicketCommentsEdit;
                    LiveObserve;
                }
            }
        }
    }
}
```

Migration:

```rust
migration!(AppSchema => AppSchemaV2, |from, to, m| {
    m.move_field(
        from.users.name,
        to.profiles.name,
    );

    m.move_field(
        from.users.email,
        to.contacts.primary_email,
    );

    m.move_field(
        from.users.phone,
        to.contacts.phone,
    );
});
```

Expected validation:

```text
ACCESS
  CustomerContactRead    preserved
  CustomerContactEdit    preserved
  TicketRead             preserved
  TicketStatusSet        preserved
  TicketCommentsEdit     preserved
  LiveObserve            preserved

ROLES
  support.l1             unchanged
  support.l2             unchanged

authorization approval required: NO
```

---

# 38. Финальный Python DX полностью

```python
class AppSchema(Schema):
    users = DbSet(User)
    tickets = DbSet(Ticket)

    @access
    def Access(s):
        CustomerContactRead = s.capability(
            "access.customer.contact.read"
        ).read(
            s.users.name,
            s.users.email,
            s.users.phone,
        )

        CustomerContactEdit = s.capability(
            "access.customer.contact.edit"
        ).write(
            s.users.email,
            s.users.phone,
        )

        TicketRead = s.capability(
            "access.ticket.read"
        ).read(
            s.tickets.id,
            s.tickets.subject,
            s.tickets.status,
        )

        TicketStatusSet = s.capability(
            "access.ticket.status.set"
        ).write(
            s.tickets.status,
        )

        SupportL1 = s.role(
            "support.l1",
            CustomerContactRead,
            TicketRead,
            TicketStatusSet,
        )

        SupportL2 = s.role(
            "support.l2",
            CustomerContactRead,
            CustomerContactEdit,
            TicketRead,
            TicketStatusSet,
        )
```

Создание:

```python
db = Database.builder("app.cfmd").create_authoritative(AppSchema)
```

Session:

```python
session = db.session_for_roles(
    principal,
    ["support.l1"],
)
```

Context:

```python
with db.context(AppSchema, session=session) as ctx:
    ...
```

---

# 39. Финальный Python migration

```python
class AppSchemaV2(Schema):
    profiles = DbSet(Profile)
    contacts = DbSet(Contact)
    tickets = DbSet(TicketV2)

    @access
    def Access(s):
        CustomerContactRead = s.capability(
            "access.customer.contact.read"
        ).read(
            s.profiles.name,
            s.contacts.primary_email,
            s.contacts.phone,
        )

        CustomerContactEdit = s.capability(
            "access.customer.contact.edit"
        ).write(
            s.contacts.primary_email,
            s.contacts.phone,
        )

        TicketRead = s.capability(
            "access.ticket.read"
        ).read(
            s.tickets.id,
            s.tickets.subject,
            s.tickets.status,
        )

        TicketStatusSet = s.capability(
            "access.ticket.status.set"
        ).write(
            s.tickets.status,
        )

        s.role(
            "support.l1",
            CustomerContactRead,
            TicketRead,
            TicketStatusSet,
        )

        s.role(
            "support.l2",
            CustomerContactRead,
            CustomerContactEdit,
            TicketRead,
            TicketStatusSet,
        )
```

```python
with migration(AppSchema, AppSchemaV2) as m:
    m.move(
        m.source.users.name,
        m.target.profiles.name,
    )

    m.move(
        m.source.users.email,
        m.target.contacts.primary_email,
    )

    m.move(
        m.source.users.phone,
        m.target.contacts.phone,
    )
```

Никакого отдельного:

```python
m.authorization(...)
```

для обычного identity/move transport.

---

# 40. Выбранный итог

Зафиксировать следующий продуктовый закон:

```text
1. Access является member authoritative Schema contract.

2. Public Access definitions могут обращаться только к DbSet/EntitySet
   этой конкретной Schema.

3. Role ссылается только на stable capabilities.

4. Capability realization ссылается только на Schema-owned coordinates.

5. Migration работает над теми же source/target Schema coordinates.

6. Migration automatically preserves only proven exact authorization identity.

7. Access widening/narrowing/shape-change является отдельным policy diff
   и требует explicit acknowledgement.

8. Target Schema всегда self-contained и содержит свой current Access,
   потому что должна поддерживать fresh authoritative create.

9. Principal -> RoleId assignment остаётся внешним identity state,
   а не частью Schema.

10. Runtime остаётся прежним:
    Role -> Capability -> exact PermissionSet -> P503-P515 checks.

11. Отдельный attachable AuthorizationPolicy исчезает из основного DX;
    он может сохраниться только как internal lowered representation.

12. Для больших role catalogs допускается source partitioning,
    но не independent policy authority.
```

Главный mental model разработчика:

```text
Schema описывает весь authoritative мир БД:

данные,
инварианты,
и кто что может с этим миром делать.
```

А migration:

```text
не только переносит данные A -> B,
но и доказывает, что security contract A -> B
остался тем же либо явно изменён.
```

Это и есть выбранный Schema-Owned Access DX.

---

# PASS562 addendum — whole-database administration after FORMAT V1

После freeze FORMAT V1 global DB operations не получают новые persisted `PermissionCoordinate` tags. Вместо этого они остаются first-class members `Schema.Access` через well-known stable capability identities: export, restore, authority-transfer, persistence-transition и protection-reconfigure. Эти capability IDs и role edges уже входят в released V1 Access representation и проходят через существующий migration Access diff/approval.

Runtime текущей authoritative Schema компилирует их в `Permission::DatabaseAdministration(...)`. Обычные `Read` / `Write`, `ModelRead` и `SchemaMigrate` не являются их superset. Restricted backup/restore проверяют admin authority до staging/I/O; raw embedded `Database` остаётся unrestricted local capability согласно существующему product law. Это не отдельная ACL-система и не backend routing.
