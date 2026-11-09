# kubebuilder

## Créer le cluster

```
  kind create cluster --name hello
```

## Créer le controleur/operateur memcached go

dans un terminal et dans le repo du projet :
```
  make install
  make run
```

dans un autre :

```
  kubectl apply -k memcached-operator/config/samples/
```

## Créer le controleur/operateur memcached rust

dans un terminal et dans le repo du projet :

```
  cargo run --bin crdgen | kubectl apply -f - # créer le CRD
  cargo run --bin memcached-controller-rs
```

dans un autre :

```
  kubectl apply -f config/samples/cache_v1alpha1_memcached.yaml # créer le controller
```

 ## Inspirations
 * https://github.com/kube-rs/kube/blob/main/examples/configmapgen_controller.rs
